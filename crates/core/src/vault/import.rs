//! Credential import (ADR-0016, spec §10).
//!
//! Parse an export file into normalized [`ImportRecord`]s, then encrypt them with
//! the user key and bulk-create them via `POST {api}/ciphers/import`. Two formats
//! to start (file-based; direct browser extraction is P2-18):
//!
//! - **Generic CSV** — header-driven, so it accepts Chrome/Edge, Firefox, Safari,
//!   and Bitwarden CSV exports by recognizing their column names.
//! - **Bitwarden JSON** — an unencrypted `.json` export.
//!
//! Parsing + request-building are pure (unit-tested with fixtures + a mock
//! transport); only [`VaultClient::import`] touches the network. **Folders are
//! not preserved yet** — every item imports to the root (filed as a follow-up,
//! not silent); the value here is getting the credentials in.

use crate::vault::auth::{Session, VaultClient};
use crate::vault::crypto::{CryptoError, SymmetricKey};
use crate::vault::http::{HttpRequest, HttpTransport};
use crate::vault::lookup::host_from_uri;
use crate::vault::sync::{Login, SyncError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A normalized credential to import, before encryption.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportRecord {
    pub name: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub uri: Option<String>,
    pub totp: Option<String>,
    pub notes: Option<String>,
}

/// A parse failure (the network path reuses [`SyncError`]).
#[derive(Debug)]
pub enum ImportError {
    /// The file could not be read as the claimed format.
    Malformed(String),
    /// The file parsed but contained no usable login rows.
    Empty,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Malformed(m) => write!(f, "malformed import file: {m}"),
            ImportError::Empty => write!(f, "no importable logins found"),
        }
    }
}

impl std::error::Error for ImportError {}

// --- Merge (de-duplication against the existing vault) ----------------------

/// The records from `records` that are **not** already in `existing` — the merge
/// filter. A record duplicates a login when they share a username **and** a URI
/// host (or, when the record has no URI, the same name). Add-only: it never
/// overwrites an existing login, so re-importing the same file is idempotent and
/// importing a second source only adds what's new.
pub fn filter_new(existing: &[Login], records: &[ImportRecord]) -> Vec<ImportRecord> {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for login in existing {
        let hosts: Vec<String> = login
            .uris
            .iter()
            .filter_map(|u| host_from_uri(&u.uri))
            .collect();
        seen.extend(identity_keys(
            &login.name,
            login.username.as_deref(),
            &hosts,
        ));
    }
    records
        .iter()
        .filter(|r| {
            let hosts: Vec<String> = r
                .uri
                .as_deref()
                .and_then(host_from_uri)
                .into_iter()
                .collect();
            !identity_keys(&r.name, r.username.as_deref(), &hosts)
                .into_iter()
                .any(|key| seen.contains(&key))
        })
        .cloned()
        .collect()
}

/// Normalized `(host-or-name, username)` identity keys — one per URI host, or a
/// single name-based key when there is no URI. Lower-cased so matching ignores case.
fn identity_keys(name: &str, username: Option<&str>, hosts: &[String]) -> Vec<(String, String)> {
    let user = username.unwrap_or("").trim().to_lowercase();
    if hosts.is_empty() {
        vec![(name.trim().to_lowercase(), user)]
    } else {
        hosts
            .iter()
            .map(|h| (h.to_lowercase(), user.clone()))
            .collect()
    }
}

// --- CSV -------------------------------------------------------------------

/// Header candidates for each logical column (lower-cased, matched exactly).
const URL_COLS: &[&str] = &["url", "uri", "login_uri", "website", "web site", "urls"];
const USERNAME_COLS: &[&str] = &["username", "login_username", "user", "email", "login_email"];
const PASSWORD_COLS: &[&str] = &["password", "login_password", "pass"];
const NAME_COLS: &[&str] = &["name", "title", "account"];
const TOTP_COLS: &[&str] = &["totp", "login_totp", "otpauth", "otp"];
const NOTES_COLS: &[&str] = &["notes", "note", "comments"];

/// Parse a CSV export. Columns are located by header name (case-insensitive), so
/// the same parser handles Chrome/Firefox/Safari/Bitwarden layouts.
pub fn parse_csv(data: &str) -> Result<Vec<ImportRecord>, ImportError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(data.as_bytes());

    let headers = reader
        .headers()
        .map_err(|e| ImportError::Malformed(e.to_string()))?
        .clone();
    let find = |cols: &[&str]| -> Option<usize> {
        headers
            .iter()
            .position(|h| cols.contains(&h.trim().to_lowercase().as_str()))
    };
    let (url_i, user_i, pass_i, name_i, totp_i, notes_i) = (
        find(URL_COLS),
        find(USERNAME_COLS),
        find(PASSWORD_COLS),
        find(NAME_COLS),
        find(TOTP_COLS),
        find(NOTES_COLS),
    );

    let mut records = Vec::new();
    for row in reader.records() {
        let row = row.map_err(|e| ImportError::Malformed(e.to_string()))?;
        let cell = |i: Option<usize>| -> Option<String> {
            i.and_then(|i| row.get(i))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let username = cell(user_i);
        let password = cell(pass_i);
        // A row with neither a username nor a password is not a credential.
        if username.is_none() && password.is_none() {
            continue;
        }
        let uri = cell(url_i);
        let name = cell(name_i)
            .or_else(|| uri.as_deref().and_then(crate::vault::lookup::host_from_uri))
            .unwrap_or_default();
        records.push(ImportRecord {
            name,
            username,
            password,
            uri,
            totp: cell(totp_i),
            notes: cell(notes_i),
        });
    }

    if records.is_empty() {
        Err(ImportError::Empty)
    } else {
        Ok(records)
    }
}

// --- Bitwarden JSON export -------------------------------------------------

#[derive(Deserialize)]
struct BwExport {
    #[serde(default)]
    items: Vec<BwItem>,
}

#[derive(Deserialize)]
struct BwItem {
    #[serde(rename = "type", default)]
    ty: u32,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    login: Option<BwLogin>,
}

#[derive(Deserialize)]
struct BwLogin {
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    totp: Option<String>,
    #[serde(default)]
    uris: Option<Vec<BwUri>>,
}

#[derive(Deserialize)]
struct BwUri {
    #[serde(default)]
    uri: Option<String>,
}

/// Parse an unencrypted Bitwarden JSON export (login items only).
pub fn parse_bitwarden_json(data: &str) -> Result<Vec<ImportRecord>, ImportError> {
    let export: BwExport =
        serde_json::from_str(data).map_err(|e| ImportError::Malformed(e.to_string()))?;

    let mut records = Vec::new();
    for item in export.items {
        if item.ty != 1 {
            continue; // login items only
        }
        let Some(login) = item.login else { continue };
        let uri = login
            .uris
            .and_then(|uris| uris.into_iter().find_map(|u| u.uri))
            .filter(|u| !u.is_empty());
        records.push(ImportRecord {
            name: item.name.unwrap_or_default(),
            username: nonempty(login.username),
            password: nonempty(login.password),
            uri,
            totp: nonempty(login.totp),
            notes: nonempty(item.notes),
        });
    }

    if records.is_empty() {
        Err(ImportError::Empty)
    } else {
        Ok(records)
    }
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.filter(|s| !s.is_empty())
}

// --- Encrypt + upload ------------------------------------------------------

#[derive(Serialize)]
struct ImportBody {
    ciphers: Vec<CipherEntry>,
    folders: Vec<()>,
    #[serde(rename = "folderRelationships")]
    folder_relationships: Vec<()>,
}

#[derive(Serialize)]
struct CipherEntry {
    #[serde(rename = "type")]
    ty: u32,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
    login: LoginEntry,
    #[serde(rename = "folderId")]
    folder_id: Option<String>,
}

#[derive(Serialize)]
struct LoginEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    totp: Option<String>,
    uris: Vec<UriEntry>,
}

#[derive(Serialize)]
struct UriEntry {
    uri: String,
    #[serde(rename = "match")]
    match_type: Option<u32>,
}

/// Bearer + JSON headers for an authenticated request.
fn auth_json_headers(access_token: &str) -> Vec<(String, String)> {
    let mut headers = vec![
        (
            "Authorization".to_string(),
            format!("Bearer {access_token}"),
        ),
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Accept".to_string(), "application/json".to_string()),
    ];
    headers.extend(crate::vault::auth::client_headers());
    headers
}

/// Encrypt one record into a Bitwarden cipher (type 1 = login). Shared by bulk
/// import and single create/update.
fn cipher_entry(key: &SymmetricKey, record: &ImportRecord) -> Result<CipherEntry, CryptoError> {
    let enc =
        |s: &str| -> Result<String, CryptoError> { Ok(key.encrypt(s.as_bytes())?.to_string()) };
    let enc_opt = |s: &Option<String>| -> Result<Option<String>, CryptoError> {
        match s {
            Some(s) if !s.is_empty() => Ok(Some(enc(s)?)),
            _ => Ok(None),
        }
    };
    let uris = match &record.uri {
        Some(u) if !u.is_empty() => vec![UriEntry {
            uri: enc(u)?,
            match_type: None,
        }],
        _ => Vec::new(),
    };
    let name = if record.name.is_empty() {
        "Untitled login"
    } else {
        &record.name
    };
    Ok(CipherEntry {
        ty: 1,
        name: enc(name)?,
        notes: enc_opt(&record.notes)?,
        login: LoginEntry {
            username: enc_opt(&record.username)?,
            password: enc_opt(&record.password)?,
            totp: enc_opt(&record.totp)?,
            uris,
        },
        folder_id: None,
    })
}

/// Encrypt records with the user key and build the `POST /ciphers/import` request.
pub fn build_import_request(
    config: &crate::vault::auth::ServerConfig,
    access_token: &str,
    key: &SymmetricKey,
    records: &[ImportRecord],
) -> Result<HttpRequest, CryptoError> {
    let ciphers = records
        .iter()
        .map(|r| cipher_entry(key, r))
        .collect::<Result<Vec<_>, _>>()?;
    let body = serde_json::to_vec(&ImportBody {
        ciphers,
        folders: vec![],
        folder_relationships: vec![],
    })
    .expect("import body is always serializable");
    Ok(HttpRequest::post(
        format!("{}/ciphers/import", config.api_url),
        auth_json_headers(access_token),
        body,
    ))
}

/// Build `POST /ciphers` to create a single login.
pub fn build_create_request(
    config: &crate::vault::auth::ServerConfig,
    access_token: &str,
    key: &SymmetricKey,
    record: &ImportRecord,
) -> Result<HttpRequest, CryptoError> {
    let body = serde_json::to_vec(&cipher_entry(key, record)?).expect("cipher is serializable");
    Ok(HttpRequest::post(
        format!("{}/ciphers", config.api_url),
        auth_json_headers(access_token),
        body,
    ))
}

/// Build `PUT /ciphers/{id}` to update an existing login.
pub fn build_update_request(
    config: &crate::vault::auth::ServerConfig,
    access_token: &str,
    key: &SymmetricKey,
    id: &str,
    record: &ImportRecord,
) -> Result<HttpRequest, CryptoError> {
    let body = serde_json::to_vec(&cipher_entry(key, record)?).expect("cipher is serializable");
    Ok(HttpRequest::put(
        format!("{}/ciphers/{id}", config.api_url),
        auth_json_headers(access_token),
        body,
    ))
}

/// Build `DELETE /ciphers/{id}` to delete a login.
pub fn build_delete_request(
    config: &crate::vault::auth::ServerConfig,
    access_token: &str,
    id: &str,
) -> HttpRequest {
    HttpRequest::delete(
        format!("{}/ciphers/{id}", config.api_url),
        auth_json_headers(access_token),
    )
}

impl<T: HttpTransport> VaultClient<T> {
    /// Encrypt and bulk-upload records to the vault. Needs an unlocked session.
    pub fn import(&self, session: &Session, records: &[ImportRecord]) -> Result<(), SyncError> {
        let request = build_import_request(
            &self.config,
            &session.tokens.access_token,
            &session.user_key,
            records,
        )
        .map_err(SyncError::Crypto)?;
        self.send_expecting_2xx(request)
    }

    /// Create a single login (`POST /ciphers`). Needs an unlocked session.
    pub fn create_login(&self, session: &Session, record: &ImportRecord) -> Result<(), SyncError> {
        let request = build_create_request(
            &self.config,
            &session.tokens.access_token,
            &session.user_key,
            record,
        )
        .map_err(SyncError::Crypto)?;
        self.send_expecting_2xx(request)
    }

    /// Update login `id` (`PUT /ciphers/{id}`). Needs an unlocked session.
    pub fn update_login(
        &self,
        session: &Session,
        id: &str,
        record: &ImportRecord,
    ) -> Result<(), SyncError> {
        let request = build_update_request(
            &self.config,
            &session.tokens.access_token,
            &session.user_key,
            id,
            record,
        )
        .map_err(SyncError::Crypto)?;
        self.send_expecting_2xx(request)
    }

    /// Delete login `id` (`DELETE /ciphers/{id}`). Needs an unlocked session.
    pub fn delete_login(&self, session: &Session, id: &str) -> Result<(), SyncError> {
        let request = build_delete_request(&self.config, &session.tokens.access_token, id);
        self.send_expecting_2xx(request)
    }

    /// Send a request and map anything but a 2xx to a `SyncError`.
    fn send_expecting_2xx(&self, request: HttpRequest) -> Result<(), SyncError> {
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::auth::ServerConfig;
    use crate::vault::crypto::EncString;
    use crate::vault::sync::{LoginUri, UriMatch};

    fn existing_login(name: &str, username: &str, uri: &str) -> Login {
        Login {
            id: name.to_string(),
            name: name.to_string(),
            username: Some(username.to_string()),
            password: Some("existing".to_string()),
            uris: if uri.is_empty() {
                vec![]
            } else {
                vec![LoginUri {
                    uri: uri.to_string(),
                    match_type: UriMatch::Domain,
                }]
            },
            totp: None,
            notes: None,
            folder: None,
        }
    }

    fn record(name: &str, username: &str, uri: &str) -> ImportRecord {
        ImportRecord {
            name: name.to_string(),
            username: Some(username.to_string()),
            password: Some("new".to_string()),
            uri: if uri.is_empty() {
                None
            } else {
                Some(uri.to_string())
            },
            totp: None,
            notes: None,
        }
    }

    #[test]
    fn filter_new_skips_host_username_duplicates_case_insensitively() {
        let existing = vec![existing_login(
            "Example",
            "alice",
            "https://example.com/login",
        )];
        let records = vec![
            // same host + username (different case, different path) → duplicate
            record("Example dup", "ALICE", "https://Example.com/account"),
            // same host, different username → new
            record("Example bob", "bob", "https://example.com"),
            // different host → new
            record("Other", "alice", "https://other.com"),
        ];
        let new = filter_new(&existing, &records);
        let names: Vec<_> = new.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["Example bob", "Other"]);
    }

    #[test]
    fn filter_new_uses_name_when_no_uri() {
        let existing = vec![existing_login("My Router", "admin", "")];
        let records = vec![
            record("my router", "admin", ""), // name+username match → duplicate
            record("My Router", "root", ""),  // same name, different user → new
        ];
        assert_eq!(filter_new(&existing, &records).len(), 1);
        assert_eq!(
            filter_new(&existing, &records)[0].username.as_deref(),
            Some("root")
        );
    }

    #[test]
    fn filter_new_into_empty_vault_keeps_everything() {
        let records = vec![
            record("A", "a", "https://a.com"),
            record("B", "b", "https://b.com"),
        ];
        assert_eq!(filter_new(&[], &records).len(), 2);
    }

    fn key() -> SymmetricKey {
        let mut b = [0u8; 64];
        for (i, x) in b.iter_mut().enumerate() {
            *x = (i as u8) ^ 0x5a;
        }
        SymmetricKey::from_bytes(&b).unwrap()
    }

    #[test]
    fn parse_csv_chrome_layout() {
        let csv = "name,url,username,password\nExample,https://example.com,alice,pw123\n";
        let records = parse_csv(csv).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "Example");
        assert_eq!(records[0].uri.as_deref(), Some("https://example.com"));
        assert_eq!(records[0].username.as_deref(), Some("alice"));
        assert_eq!(records[0].password.as_deref(), Some("pw123"));
    }

    #[test]
    fn parse_csv_preserves_specials_via_quoting() {
        // A password containing a comma, a quote, and a newline — the exact case a
        // naive split would corrupt.
        let csv = "url,username,password\nhttps://x.com,bob,\"p,a\"\"ss\nword\"\n";
        let records = parse_csv(csv).unwrap();
        assert_eq!(records[0].password.as_deref(), Some("p,a\"ss\nword"));
    }

    #[test]
    fn parse_csv_bitwarden_and_safari_headers() {
        let bw = "folder,name,login_uri,login_username,login_password,login_totp\n,Site,https://s.com,u,p,otp\n";
        let r = parse_csv(bw).unwrap();
        assert_eq!(r[0].username.as_deref(), Some("u"));
        assert_eq!(r[0].totp.as_deref(), Some("otp"));

        let safari = "Title,URL,Username,Password\nMy Bank,https://bank.com,carol,secret\n";
        let r = parse_csv(safari).unwrap();
        assert_eq!(r[0].name, "My Bank");
        assert_eq!(r[0].password.as_deref(), Some("secret"));
    }

    #[test]
    fn parse_csv_skips_rows_without_credentials_and_derives_name() {
        let csv = "url,username,password\nhttps://ok.com,u,p\nhttps://empty.com,,\n";
        let r = parse_csv(csv).unwrap();
        assert_eq!(r.len(), 1, "the credential-less row is skipped");
        // No name column → derived from the URL host.
        assert_eq!(r[0].name, "ok.com");
    }

    #[test]
    fn parse_csv_empty_is_error() {
        assert!(matches!(
            parse_csv("url,username,password\n"),
            Err(ImportError::Empty)
        ));
    }

    #[test]
    fn parse_bitwarden_json_logins_only() {
        let json = r#"{"encrypted":false,"items":[
            {"type":1,"name":"A","login":{"username":"u","password":"p","uris":[{"uri":"https://a.com"}]}},
            {"type":2,"name":"a note","secureNote":{"type":0}}
        ]}"#;
        let r = parse_bitwarden_json(json).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "A");
        assert_eq!(r[0].uri.as_deref(), Some("https://a.com"));
    }

    #[test]
    fn build_import_request_encrypts_every_field() {
        let k = key();
        let records = vec![ImportRecord {
            name: "Example".to_string(),
            username: Some("alice".to_string()),
            password: Some("s3cret".to_string()),
            uri: Some("https://example.com".to_string()),
            totp: None,
            notes: None,
        }];
        let req =
            build_import_request(&ServerConfig::bitwarden_cloud(), "tok", &k, &records).unwrap();
        assert_eq!(req.url, "https://api.bitwarden.com/ciphers/import");
        let auth = req
            .headers
            .iter()
            .find(|(h, _)| h == "Authorization")
            .map(|(_, v)| v.clone());
        assert_eq!(auth.as_deref(), Some("Bearer tok"));

        // Decrypt the built body back and confirm the fields survived encryption.
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        let cipher = &body["ciphers"][0];
        let dec = |v: &serde_json::Value| {
            let s = v.as_str().unwrap();
            String::from_utf8(k.decrypt(&EncString::parse(s).unwrap()).unwrap()).unwrap()
        };
        assert_eq!(dec(&cipher["name"]), "Example");
        assert_eq!(dec(&cipher["login"]["username"]), "alice");
        assert_eq!(dec(&cipher["login"]["password"]), "s3cret");
        assert_eq!(
            dec(&cipher["login"]["uris"][0]["uri"]),
            "https://example.com"
        );
        // Absent optional fields are omitted, not encrypted-empty.
        assert!(cipher["login"]["totp"].is_null());
        assert!(cipher["notes"].is_null());
    }

    #[test]
    fn client_import_reports_server_and_auth_errors() {
        use crate::vault::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
        use std::cell::RefCell;

        struct Once(RefCell<Option<HttpResponse>>);
        impl HttpTransport for Once {
            fn send(&self, _r: HttpRequest) -> Result<HttpResponse, TransportError> {
                Ok(self.0.borrow_mut().take().unwrap())
            }
        }
        let session = Session {
            tokens: crate::vault::auth::Tokens {
                access_token: "t".to_string(),
                refresh_token: String::new(),
                expires_in: 1,
            },
            user_key: key(),
            kdf: crate::vault::crypto::Kdf::Pbkdf2 { iterations: 1 },
            email: "u@e.com".to_string(),
            protected_key: String::new(),
        };
        let records = [ImportRecord {
            name: "x".to_string(),
            password: Some("p".to_string()),
            ..Default::default()
        }];

        let ok = VaultClient::new(
            Once(RefCell::new(Some(HttpResponse {
                status: 200,
                body: b"[]".to_vec(),
            }))),
            ServerConfig::bitwarden_cloud(),
        );
        assert!(ok.import(&session, &records).is_ok());

        let unauthorized = VaultClient::new(
            Once(RefCell::new(Some(HttpResponse {
                status: 401,
                body: vec![],
            }))),
            ServerConfig::bitwarden_cloud(),
        );
        assert!(matches!(
            unauthorized.import(&session, &records),
            Err(SyncError::Unauthorized)
        ));
    }
}
