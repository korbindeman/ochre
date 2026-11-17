//! Vault sync + cipher model (ADR-0016, spec §10).
//!
//! `GET {api}/sync` returns the whole vault with every field still an
//! [`EncString`]; this module decrypts the **login** ciphers into a plain
//! [`Login`] model the rest of the browser can use - `vault_lookup` (P2-11),
//! autofill (P2-15), the UI (P2-13).
//!
//! ## Which key decrypts a cipher
//!
//! Not always the user key. Modern Bitwarden clients use **cipher key
//! encryption**: a cipher may carry its own `key` - a fresh symmetric key,
//! itself an [`EncString`] wrapped by the user key - and *that* key, not the
//! user key, encrypts the cipher's fields. So per cipher we unwrap `key` (if
//! present) and decrypt the fields with it, else fall back to the user key.
//! Folder names are always under the user key. Without this, any cipher saved by
//! a recent client fails MAC verification - the whole reason a live sync would
//! error with `crypto error: MAC verification failed`.
//!
//! **Organization** ciphers are encrypted under the org's key (RSA-wrapped in
//! the sync profile), a separate key path not yet implemented (backlog P2-27);
//! they are skipped so one org item can't fail the entire personal sync.
//!
//! Only login ciphers (`type == 1`) are modelled; cards / notes / identities are
//! skipped until a feature needs them. Response parsing + decryption are pure
//! ([`parse_sync`]); only [`VaultClient::sync`] touches the network.

use crate::vault::auth::{Session, VaultClient};
use crate::vault::crypto::{CryptoError, EncString, SymmetricKey};
use crate::vault::http::{HttpRequest, HttpTransport};
use serde::Deserialize;
use std::collections::HashMap;

/// A decrypted vault - the login items, ready to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vault {
    pub logins: Vec<Login>,
}

/// A decrypted login cipher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub id: String,
    pub name: String,
    pub username: Option<String>,
    pub password: Option<String>,
    /// Login URIs + their per-URI match rules (used by `vault_lookup`).
    pub uris: Vec<LoginUri>,
    /// TOTP secret / otpauth URI, if the cipher carries one.
    pub totp: Option<String>,
    /// Free-form notes on the cipher (needed for round-trip editing).
    pub notes: Option<String>,
    /// Decrypted folder name, if the cipher is filed in one.
    pub folder: Option<String>,
}

/// A login URI plus how Bitwarden should match it against a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginUri {
    pub uri: String,
    pub match_type: UriMatch,
}

/// Bitwarden's per-URI match rule (`UriMatchType`). `Domain` is the default when
/// a cipher URI carries no explicit rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UriMatch {
    Domain,
    Host,
    StartsWith,
    Exact,
    RegEx,
    Never,
}

impl UriMatch {
    /// Map the wire int (null → `Domain`; unknown → `Domain`, the safe default).
    fn from_raw(n: Option<u32>) -> Self {
        match n {
            Some(1) => UriMatch::Host,
            Some(2) => UriMatch::StartsWith,
            Some(3) => UriMatch::Exact,
            Some(4) => UriMatch::RegEx,
            Some(5) => UriMatch::Never,
            _ => UriMatch::Domain,
        }
    }
}

/// What can go wrong syncing.
#[derive(Debug)]
pub enum SyncError {
    /// Network / TLS / connection failure.
    Transport(String),
    /// The access token was rejected (expired - refresh and retry).
    Unauthorized,
    /// An HTTP error the flow doesn't model specifically.
    Server { status: u16, message: String },
    /// The sync body could not be parsed / a field was not valid UTF-8.
    Parse(String),
    /// A cipher field failed to decrypt (wrong key / tampered).
    Crypto(CryptoError),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::Transport(m) => write!(f, "transport error: {m}"),
            SyncError::Unauthorized => write!(f, "unauthorized (token expired)"),
            SyncError::Server { status, message } => write!(f, "server error {status}: {message}"),
            SyncError::Parse(m) => write!(f, "parse error: {m}"),
            SyncError::Crypto(e) => write!(f, "crypto error: {e}"),
        }
    }
}

impl std::error::Error for SyncError {}

impl<T: HttpTransport> VaultClient<T> {
    /// Fetch the raw (still-encrypted) `/sync` body for an unlocked session. The
    /// caller decrypts it with [`parse_sync`]; the manager also caches this blob
    /// verbatim (it is encrypted at rest - every field is an `EncString`).
    pub fn sync_raw(&self, session: &Session) -> Result<Vec<u8>, SyncError> {
        let mut headers = vec![
            (
                "Authorization".to_string(),
                format!("Bearer {}", session.tokens.access_token),
            ),
            ("Accept".to_string(), "application/json".to_string()),
        ];
        headers.extend(crate::vault::auth::client_headers());
        let request = HttpRequest::get(format!("{}/sync", self.config.api_url), headers);
        let response = self
            .transport
            .send(request)
            .map_err(|e| SyncError::Transport(e.0))?;
        if response.status == 401 {
            return Err(SyncError::Unauthorized);
        }
        if !(200..300).contains(&response.status) {
            let message = String::from_utf8_lossy(&response.body).trim().to_string();
            return Err(SyncError::Server {
                status: response.status,
                message: if message.is_empty() {
                    format!("HTTP {}", response.status)
                } else {
                    message
                },
            });
        }
        Ok(response.body)
    }

    /// Fetch and decrypt the vault for an unlocked session.
    pub fn sync(&self, session: &Session) -> Result<Vault, SyncError> {
        let body = self.sync_raw(session)?;
        parse_sync(&body, &session.user_key)
    }
}

// --- Raw wire model (fields are EncStrings; casing varies, so alias both) -----

#[derive(Deserialize)]
struct SyncResponse {
    #[serde(rename = "ciphers", alias = "Ciphers", default)]
    ciphers: Vec<RawCipher>,
    #[serde(rename = "folders", alias = "Folders", default)]
    folders: Vec<RawFolder>,
}

#[derive(Deserialize)]
struct RawFolder {
    #[serde(rename = "id", alias = "Id")]
    id: String,
    #[serde(rename = "name", alias = "Name")]
    name: String,
}

#[derive(Deserialize)]
struct RawCipher {
    #[serde(rename = "id", alias = "Id")]
    id: String,
    #[serde(rename = "type", alias = "Type")]
    ty: u32,
    #[serde(rename = "name", alias = "Name", default)]
    name: Option<String>,
    #[serde(rename = "notes", alias = "Notes", default)]
    notes: Option<String>,
    #[serde(rename = "folderId", alias = "FolderId", default)]
    folder_id: Option<String>,
    /// Per-cipher key (cipher key encryption), an `EncString` wrapped by the user
    /// key. When present, it - not the user key - decrypts this cipher's fields.
    #[serde(rename = "key", alias = "Key", default)]
    key: Option<String>,
    /// Set when the cipher belongs to an organization (org-key path, not the user
    /// key). We don't decrypt these yet, so their presence means "skip".
    #[serde(rename = "organizationId", alias = "OrganizationId", default)]
    organization_id: Option<String>,
    #[serde(rename = "login", alias = "Login", default)]
    login: Option<RawLogin>,
}

#[derive(Deserialize)]
struct RawLogin {
    #[serde(rename = "username", alias = "Username", default)]
    username: Option<String>,
    #[serde(rename = "password", alias = "Password", default)]
    password: Option<String>,
    #[serde(rename = "totp", alias = "Totp", default)]
    totp: Option<String>,
    #[serde(rename = "uris", alias = "Uris", default)]
    uris: Option<Vec<RawUri>>,
}

#[derive(Deserialize)]
struct RawUri {
    #[serde(rename = "uri", alias = "Uri", default)]
    uri: Option<String>,
    #[serde(rename = "match", alias = "Match", default)]
    match_type: Option<u32>,
}

/// Cipher type 1 = Login (the only type we model).
const CIPHER_TYPE_LOGIN: u32 = 1;

/// Parse a `/sync` body and decrypt its login ciphers with the user key.
pub fn parse_sync(body: &[u8], key: &SymmetricKey) -> Result<Vault, SyncError> {
    let raw: SyncResponse =
        serde_json::from_slice(body).map_err(|e| SyncError::Parse(format!("sync: {e}")))?;

    // folder id → decrypted name, for resolving each cipher's folder.
    let mut folders: HashMap<String, String> = HashMap::with_capacity(raw.folders.len());
    for folder in &raw.folders {
        folders.insert(folder.id.clone(), decrypt_string(key, &folder.name)?);
    }

    let mut logins = Vec::new();
    for cipher in raw.ciphers {
        if cipher.ty != CIPHER_TYPE_LOGIN {
            continue;
        }
        // Org ciphers ride the organization key (not the user key) - unsupported
        // for now (P2-27). Skip rather than fail the whole sync on one of them.
        if cipher.organization_id.is_some() {
            continue;
        }
        let Some(login) = cipher.login else { continue };

        // Cipher key encryption: if the cipher carries its own `key`, unwrap it
        // with the user key and decrypt this cipher's fields with it; otherwise
        // the fields are under the user key directly.
        let cipher_key = unwrap_cipher_key(key, cipher.key.as_deref())?;
        let item_key = cipher_key.as_ref().unwrap_or(key);

        let name = match cipher.name.as_deref() {
            Some(n) if !n.is_empty() => decrypt_string(item_key, n)?,
            _ => String::new(),
        };
        let uris = match login.uris {
            Some(uris) => {
                let mut out = Vec::with_capacity(uris.len());
                for u in uris {
                    if let Some(uri) = opt_decrypt(item_key, u.uri.as_deref())? {
                        out.push(LoginUri {
                            uri,
                            match_type: UriMatch::from_raw(u.match_type),
                        });
                    }
                }
                out
            }
            None => Vec::new(),
        };
        let folder = cipher.folder_id.and_then(|id| folders.get(&id).cloned());

        logins.push(Login {
            id: cipher.id,
            name,
            username: opt_decrypt(item_key, login.username.as_deref())?,
            password: opt_decrypt(item_key, login.password.as_deref())?,
            uris,
            totp: opt_decrypt(item_key, login.totp.as_deref())?,
            notes: opt_decrypt(item_key, cipher.notes.as_deref())?,
            folder,
        });
    }

    Ok(Vault { logins })
}

/// Unwrap a cipher's own `key` (cipher key encryption) with the user key, if it
/// carries one. Absent / empty → `None` (fields are under the user key directly).
fn unwrap_cipher_key(
    user_key: &SymmetricKey,
    cipher_key: Option<&str>,
) -> Result<Option<SymmetricKey>, SyncError> {
    match cipher_key {
        Some(k) if !k.is_empty() => {
            let wrapped = EncString::parse(k).map_err(SyncError::Crypto)?;
            Ok(Some(
                user_key.unwrap_key(&wrapped).map_err(SyncError::Crypto)?,
            ))
        }
        _ => Ok(None),
    }
}

/// Decrypt one EncString field to a UTF-8 string.
fn decrypt_string(key: &SymmetricKey, s: &str) -> Result<String, SyncError> {
    let enc = EncString::parse(s).map_err(SyncError::Crypto)?;
    let bytes = key.decrypt(&enc).map_err(SyncError::Crypto)?;
    String::from_utf8(bytes)
        .map_err(|_| SyncError::Parse("cipher field is not valid UTF-8".to_string()))
}

/// Decrypt an optional field; absent or empty → `None`.
fn opt_decrypt(key: &SymmetricKey, s: Option<&str>) -> Result<Option<String>, SyncError> {
    match s {
        Some(s) if !s.is_empty() => Ok(Some(decrypt_string(key, s)?)),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::auth::{ServerConfig, Tokens};
    use crate::vault::crypto::Kdf;
    use crate::vault::http::{HttpRequest, HttpResponse, TransportError};
    use std::cell::RefCell;

    /// A test user key (arbitrary bytes).
    fn key() -> SymmetricKey {
        let mut b = [0u8; 64];
        for (i, x) in b.iter_mut().enumerate() {
            *x = (i as u8).wrapping_mul(7).wrapping_add(1);
        }
        SymmetricKey::from_bytes(&b).unwrap()
    }

    /// Encrypt a value the way the server would store it.
    fn enc(key: &SymmetricKey, plaintext: &str) -> String {
        key.encrypt(plaintext.as_bytes()).unwrap().to_string()
    }

    #[test]
    fn parse_sync_decrypts_a_login_with_folder_and_uris() {
        let k = key();
        let body = format!(
            r#"{{
              "folders": [{{ "id": "f1", "name": "{fname}" }}],
              "ciphers": [{{
                "id": "c1", "type": 1, "name": "{name}", "folderId": "f1",
                "login": {{
                  "username": "{user}", "password": "{pass}", "totp": "{totp}",
                  "uris": [{{ "uri": "{uri1}" }}, {{ "uri": "{uri2}" }}]
                }}
              }}]
            }}"#,
            fname = enc(&k, "Work"),
            name = enc(&k, "Example"),
            user = enc(&k, "alice@example.com"),
            pass = enc(&k, "s3cret!"),
            totp = enc(&k, "otpauth://totp/x"),
            uri1 = enc(&k, "https://example.com"),
            uri2 = enc(&k, "https://login.example.com"),
        );
        let vault = parse_sync(body.as_bytes(), &k).unwrap();
        assert_eq!(vault.logins.len(), 1);
        let l = &vault.logins[0];
        assert_eq!(l.id, "c1");
        assert_eq!(l.name, "Example");
        assert_eq!(l.username.as_deref(), Some("alice@example.com"));
        assert_eq!(l.password.as_deref(), Some("s3cret!"));
        assert_eq!(l.totp.as_deref(), Some("otpauth://totp/x"));
        assert_eq!(
            l.uris.iter().map(|u| u.uri.as_str()).collect::<Vec<_>>(),
            vec!["https://example.com", "https://login.example.com"]
        );
        // No explicit match rule on the fixture URIs → the default.
        assert!(l.uris.iter().all(|u| u.match_type == UriMatch::Domain));
        assert_eq!(l.folder.as_deref(), Some("Work"));
    }

    #[test]
    fn parse_sync_skips_non_login_ciphers() {
        let k = key();
        // A secure note (type 2) and a login (type 1); only the login survives.
        let body = format!(
            r#"{{"ciphers":[
                {{"id":"note","type":2,"name":"{n}"}},
                {{"id":"login","type":1,"name":"{n2}","login":{{"password":"{p}"}}}}
            ]}}"#,
            n = enc(&k, "a note"),
            n2 = enc(&k, "site"),
            p = enc(&k, "pw"),
        );
        let vault = parse_sync(body.as_bytes(), &k).unwrap();
        assert_eq!(vault.logins.len(), 1);
        assert_eq!(vault.logins[0].id, "login");
    }

    #[test]
    fn parse_sync_handles_null_fields_and_pascal_case() {
        let k = key();
        // PascalCase keys, a null username, no uris, no folder.
        let body = format!(
            r#"{{"Ciphers":[{{"Id":"c","Type":1,"Name":"{n}","Login":{{"Username":null,"Password":"{p}"}}}}]}}"#,
            n = enc(&k, "site"),
            p = enc(&k, "pw"),
        );
        let vault = parse_sync(body.as_bytes(), &k).unwrap();
        let l = &vault.logins[0];
        assert_eq!(l.username, None);
        assert_eq!(l.password.as_deref(), Some("pw"));
        assert!(l.uris.is_empty());
        assert_eq!(l.folder, None);
    }

    #[test]
    fn parse_sync_wrong_key_is_a_crypto_error() {
        let body = format!(
            r#"{{"ciphers":[{{"id":"c","type":1,"name":"{n}","login":{{"password":"{p}"}}}}]}}"#,
            n = enc(&key(), "site"),
            p = enc(&key(), "pw"),
        );
        let mut other = [0u8; 64];
        other.fill(0x11);
        let wrong = SymmetricKey::from_bytes(&other).unwrap();
        assert!(matches!(
            parse_sync(body.as_bytes(), &wrong),
            Err(SyncError::Crypto(_))
        ));
    }

    #[test]
    fn parse_sync_decrypts_a_cipher_with_its_own_key() {
        let user = key();
        // A per-cipher key: fresh 64 bytes, wrapped by the user key exactly as the
        // server delivers `cipher.key`. The cipher's fields are under *this* key.
        let mut ckb = [0u8; 64];
        for (i, b) in ckb.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(3).wrapping_add(9);
        }
        let cipher_key = SymmetricKey::from_bytes(&ckb).unwrap();
        let wrapped = user.encrypt(&ckb).unwrap().to_string();
        let name = enc(&cipher_key, "Example");
        let user_f = enc(&cipher_key, "alice@example.com");
        let pass = enc(&cipher_key, "s3cret!");
        let body = format!(
            r#"{{"ciphers":[{{"id":"c1","type":1,"key":"{wrapped}","name":"{name}","login":{{"username":"{user_f}","password":"{pass}"}}}}]}}"#
        );
        // Given only the user key, decryption still succeeds - the cipher key is
        // unwrapped from `key` first. This is the path a live modern vault needs.
        let vault = parse_sync(body.as_bytes(), &user).unwrap();
        assert_eq!(vault.logins.len(), 1);
        let l = &vault.logins[0];
        assert_eq!(l.name, "Example");
        assert_eq!(l.username.as_deref(), Some("alice@example.com"));
        assert_eq!(l.password.as_deref(), Some("s3cret!"));
    }

    #[test]
    fn parse_sync_skips_organization_ciphers() {
        let user = key();
        // A cipher under an org key - a different key the user key can't unwrap.
        let mut other = [0u8; 64];
        other.fill(0x22);
        let org_key = SymmetricKey::from_bytes(&other).unwrap();
        let name = enc(&org_key, "Org Login");
        let pass = enc(&org_key, "pw");
        let body = format!(
            r#"{{"ciphers":[{{"id":"o1","type":1,"organizationId":"org-123","name":"{name}","login":{{"password":"{pass}"}}}}]}}"#
        );
        // Skipped, not a hard crypto error: one org item can't brick a personal sync.
        let vault = parse_sync(body.as_bytes(), &user).unwrap();
        assert!(vault.logins.is_empty());
    }

    #[test]
    fn empty_vault_parses() {
        let vault = parse_sync(br#"{"ciphers":[],"folders":[]}"#, &key()).unwrap();
        assert!(vault.logins.is_empty());
    }

    // --- VaultClient::sync over a mock transport ---

    struct MockTransport {
        response: HttpResponse,
        requests: RefCell<Vec<HttpRequest>>,
    }

    impl HttpTransport for MockTransport {
        fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.requests.borrow_mut().push(request);
            Ok(self.response.clone())
        }
    }

    fn session_with(key: SymmetricKey, token: &str) -> Session {
        Session {
            tokens: Tokens {
                access_token: token.to_string(),
                refresh_token: String::new(),
                expires_in: 3600,
            },
            user_key: key,
            kdf: Kdf::Pbkdf2 { iterations: 1 },
            email: "user@example.com".to_string(),
            protected_key: String::new(),
        }
    }

    #[test]
    fn client_sync_sends_bearer_and_decrypts() {
        let k = key();
        let body = format!(
            r#"{{"ciphers":[{{"id":"c","type":1,"name":"{n}","login":{{"password":"{p}"}}}}]}}"#,
            n = enc(&k, "site"),
            p = enc(&k, "pw"),
        );
        let transport = MockTransport {
            response: HttpResponse {
                status: 200,
                body: body.into_bytes(),
            },
            requests: RefCell::new(Vec::new()),
        };
        let client = VaultClient::new(transport, ServerConfig::bitwarden_cloud());
        let vault = client.sync(&session_with(k, "access-tok")).unwrap();
        assert_eq!(vault.logins[0].password.as_deref(), Some("pw"));

        let req = client.transport.requests.borrow()[0].clone();
        assert_eq!(req.url, "https://api.bitwarden.com/sync");
        let auth = req
            .headers
            .iter()
            .find(|(h, _)| h == "Authorization")
            .map(|(_, v)| v.clone());
        assert_eq!(auth.as_deref(), Some("Bearer access-tok"));
    }

    #[test]
    fn client_sync_maps_401_to_unauthorized() {
        let transport = MockTransport {
            response: HttpResponse {
                status: 401,
                body: b"unauthorized".to_vec(),
            },
            requests: RefCell::new(Vec::new()),
        };
        let client = VaultClient::new(transport, ServerConfig::bitwarden_cloud());
        assert!(matches!(
            client.sync(&session_with(key(), "t")),
            Err(SyncError::Unauthorized)
        ));
    }

    #[test]
    fn client_sync_maps_5xx_to_server_error() {
        let transport = MockTransport {
            response: HttpResponse {
                status: 502,
                body: b"bad gateway".to_vec(),
            },
            requests: RefCell::new(Vec::new()),
        };
        let client = VaultClient::new(transport, ServerConfig::bitwarden_cloud());
        assert!(matches!(
            client.sync(&session_with(key(), "t")),
            Err(SyncError::Server { status: 502, .. })
        ));
    }
}
