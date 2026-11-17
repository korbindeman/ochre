//! Vault lock/unlock state machine (ADR-0016).
//!
//! The stateful object the rest of the browser holds. It ties the pieces
//! together: unlock (online via [`auth`](crate::vault::auth), or offline from the
//! [`cache`](crate::vault::cache)), lock (drop the key material), and
//! [`lookup`](crate::vault::lookup) a page host against the decrypted vault.
//!
//! - **Online unlock** = login → sync → decrypt → write cache; leaves a live
//!   [`Session`] so the vault can `resync`.
//! - **Offline unlock** = re-derive the key from the cached protected key and
//!   decrypt the cached blob; read-only (no session, so no `resync` until a
//!   fresh online unlock).
//! - **Lock** drops the decrypted vault + session, so the user key (a
//!   `ZeroizeOnDrop` `SymmetricKey`) is wiped from memory; the cache stays.

use crate::vault::auth::{AuthError, Credentials, Device, Grant, Session, TwoFactor, VaultClient};
use crate::vault::cache::{self, VaultCache};
use crate::vault::crypto::{CryptoError, EncString, MasterKey};
use crate::vault::http::HttpTransport;
use crate::vault::import::ImportRecord;
use crate::vault::lookup::matching_logins;
use crate::vault::search::search_logins;
use crate::vault::sync::{parse_sync, Login, SyncError, Vault};
use crate::vault::usage::UsageMap;
use zeroize::Zeroizing;

/// Owns the vault's lock state and the client used to unlock/sync it.
pub struct VaultManager<T: HttpTransport> {
    client: VaultClient<T>,
    device: Device,
    /// Where the encrypted cache lives; `None` disables persistence (ephemeral).
    cache_path: Option<String>,
    state: State,
}

// The manager holds exactly one `State` (never a collection), so the size gap
// between `Locked` and `Unlocked` costs nothing - boxing would only add
// indirection.
#[allow(clippy::large_enum_variant)]
enum State {
    Locked,
    Unlocked {
        vault: Vault,
        /// A live session (online unlock) enables `resync`; `None` when unlocked
        /// offline from cache.
        session: Option<Session>,
    },
}

/// What can go wrong operating the vault.
#[derive(Debug)]
pub enum VaultError {
    /// The operation requires an unlocked vault (or a live session, for resync).
    Locked,
    /// Offline unlock with no cache present.
    NoCache,
    /// The master password did not decrypt the cached key.
    InvalidPassword,
    Auth(AuthError),
    Sync(SyncError),
    Crypto(CryptoError),
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultError::Locked => write!(f, "vault is locked"),
            VaultError::NoCache => write!(f, "no cached vault"),
            VaultError::InvalidPassword => write!(f, "invalid master password"),
            VaultError::Auth(e) => write!(f, "auth: {e}"),
            VaultError::Sync(e) => write!(f, "sync: {e}"),
            VaultError::Crypto(e) => write!(f, "crypto: {e}"),
        }
    }
}

impl std::error::Error for VaultError {}

impl<T: HttpTransport> VaultManager<T> {
    /// A locked manager. `cache_path` persists the encrypted vault (`None` = off).
    pub fn new(client: VaultClient<T>, device: Device, cache_path: Option<String>) -> Self {
        Self {
            client,
            device,
            cache_path,
            state: State::Locked,
        }
    }

    pub fn is_locked(&self) -> bool {
        matches!(self.state, State::Locked)
    }

    /// Log in, sync, decrypt, and cache - leaving the vault unlocked with a live
    /// session.
    pub fn unlock_online(
        &mut self,
        email: String,
        master_password: String,
        grant: Grant,
        two_factor: Option<&TwoFactor>,
    ) -> Result<(), VaultError> {
        let creds = Credentials {
            email,
            master_password: Zeroizing::new(master_password),
            grant,
        };
        let session = self
            .client
            .login(&creds, &self.device, two_factor)
            .map_err(VaultError::Auth)?;
        let raw = self.client.sync_raw(&session).map_err(VaultError::Sync)?;
        let vault = parse_sync(&raw, &session.user_key).map_err(VaultError::Sync)?;

        if let Some(path) = self.cache_path.as_deref() {
            cache::save(path, &cache_from(&session, &raw));
        }
        self.state = State::Unlocked {
            vault,
            session: Some(session),
        };
        Ok(())
    }

    /// Ask the server to email a login two-factor code (the Email provider). Does
    /// not change lock state - it just dispatches the code so the user can read it
    /// and then re-attempt [`unlock_online`](Self::unlock_online) with it.
    pub fn send_email_code(&self, email: &str, master_password: &str) -> Result<(), VaultError> {
        let creds = Credentials {
            email: email.to_string(),
            master_password: Zeroizing::new(master_password.to_string()),
            grant: Grant::Password,
        };
        self.client
            .send_email_login(&creds, &self.device)
            .map_err(VaultError::Auth)
    }

    /// Unlock read-only from the cache - re-derive the key from the master
    /// password and decrypt the cached vault. No network.
    pub fn unlock_offline(&mut self, master_password: &str) -> Result<(), VaultError> {
        let path = self.cache_path.as_deref().ok_or(VaultError::NoCache)?;
        let cache = cache::load(path).ok_or(VaultError::NoCache)?;
        let vault = decrypt_cache(&cache, master_password)?;
        self.state = State::Unlocked {
            vault,
            session: None,
        };
        Ok(())
    }

    /// Wipe the decrypted vault + session (the user key zeroizes on drop).
    pub fn lock(&mut self) {
        self.state = State::Locked;
    }

    /// Logins matching `host`, best-first (recency breaks ties within a match
    /// tier - see [`matching_logins`]). `usage` is supplied by the caller rather
    /// than read here: the manager stays a pure state machine over its own vault,
    /// and the process-global usage store lives with the other globals in
    /// [`service`](crate::vault::service). `Err(Locked)` if the vault is locked.
    pub fn lookup(&self, host: &str, usage: &UsageMap) -> Result<Vec<&Login>, VaultError> {
        match &self.state {
            State::Unlocked { vault, .. } => Ok(matching_logins(vault, host, usage)),
            State::Locked => Err(VaultError::Locked),
        }
    }

    /// Logins matching a free-text `query`, best-first (see [`search_logins`]).
    /// An empty query lists the whole vault. `Err(Locked)` if the vault is locked.
    pub fn search(&self, query: &str, usage: &UsageMap) -> Result<Vec<&Login>, VaultError> {
        match &self.state {
            State::Unlocked { vault, .. } => Ok(search_logins(vault, query, usage)),
            State::Locked => Err(VaultError::Locked),
        }
    }

    /// All decrypted logins (for the management UI). `Err(Locked)` when locked.
    pub fn logins(&self) -> Result<&[Login], VaultError> {
        match &self.state {
            State::Unlocked { vault, .. } => Ok(&vault.logins),
            State::Locked => Err(VaultError::Locked),
        }
    }

    /// Debug-only: force an unlocked, session-less state with the given logins -
    /// no network, no crypto. For the autofill end-to-end harness (the real
    /// unlock path is live-validated separately).
    #[cfg(debug_assertions)]
    pub fn unlock_with_logins(&mut self, logins: Vec<Login>) {
        self.state = State::Unlocked {
            vault: Vault { logins },
            session: None,
        };
    }

    /// **Merge**-import: refresh from the server, keep only the records not
    /// already present (dedupe against the current vault), upload those, and
    /// resync so they appear locally. Returns the count actually added (skipped
    /// duplicates are not counted). Requires a live online session - so
    /// re-importing a file is idempotent and a second source only adds what's new.
    pub fn import(&mut self, records: &[ImportRecord]) -> Result<usize, VaultError> {
        // Sync first so the merge dedupes against what's actually on the server.
        self.resync()?;
        let new_records: Vec<ImportRecord> = {
            let vault = match &self.state {
                State::Unlocked { vault, .. } => vault,
                State::Locked => return Err(VaultError::Locked),
            };
            crate::vault::import::filter_new(&vault.logins, records)
        };
        if new_records.is_empty() {
            return Ok(0);
        }
        {
            let session = match &self.state {
                State::Unlocked {
                    session: Some(session),
                    ..
                } => session,
                _ => return Err(VaultError::Locked),
            };
            self.client
                .import(session, &new_records)
                .map_err(VaultError::Sync)?;
        }
        self.resync()?;
        Ok(new_records.len())
    }

    /// Create a new login, then resync. Requires a live online session.
    pub fn add_login(&mut self, record: &ImportRecord) -> Result<(), VaultError> {
        self.with_session(|client, session| client.create_login(session, record))?;
        self.resync()
    }

    /// Update login `id`, then resync. Requires a live online session.
    pub fn update_login(&mut self, id: &str, record: &ImportRecord) -> Result<(), VaultError> {
        self.with_session(|client, session| client.update_login(session, id, record))?;
        self.resync()
    }

    /// Delete login `id`, then resync. Requires a live online session.
    pub fn delete_login(&mut self, id: &str) -> Result<(), VaultError> {
        self.with_session(|client, session| client.delete_login(session, id))?;
        self.resync()
    }

    /// Run a client op that needs the live session, mapping errors to `VaultError`.
    fn with_session(
        &self,
        op: impl FnOnce(&VaultClient<T>, &Session) -> Result<(), SyncError>,
    ) -> Result<(), VaultError> {
        let session = match &self.state {
            State::Unlocked {
                session: Some(session),
                ..
            } => session,
            _ => return Err(VaultError::Locked),
        };
        op(&self.client, session).map_err(VaultError::Sync)
    }

    /// Re-fetch the vault from the server (requires a live online session).
    pub fn resync(&mut self) -> Result<(), VaultError> {
        let (vault, cache) = {
            let session = match &self.state {
                State::Unlocked {
                    session: Some(session),
                    ..
                } => session,
                _ => return Err(VaultError::Locked),
            };
            let raw = self.client.sync_raw(session).map_err(VaultError::Sync)?;
            let vault = parse_sync(&raw, &session.user_key).map_err(VaultError::Sync)?;
            let cache = self.cache_path.as_ref().map(|_| cache_from(session, &raw));
            (vault, cache)
        };

        if let (Some(path), Some(cache)) = (self.cache_path.as_deref(), cache.as_ref()) {
            cache::save(path, cache);
        }
        if let State::Unlocked { vault: current, .. } = &mut self.state {
            *current = vault;
        }
        Ok(())
    }
}

/// Build the cache snapshot from a session + the raw sync blob.
fn cache_from(session: &Session, raw: &[u8]) -> VaultCache {
    VaultCache {
        email: session.email.clone(),
        kdf: session.kdf,
        protected_key: session.protected_key.clone(),
        encrypted_vault: String::from_utf8_lossy(raw).into_owned(),
    }
}

/// Re-derive the user key from the master password and decrypt the cached vault.
fn decrypt_cache(cache: &VaultCache, master_password: &str) -> Result<Vault, VaultError> {
    let master =
        MasterKey::derive(master_password, &cache.email, cache.kdf).map_err(VaultError::Crypto)?;
    let stretched = master.stretch().map_err(VaultError::Crypto)?;
    let protected = EncString::parse(&cache.protected_key).map_err(VaultError::Crypto)?;
    let user_key = stretched
        .unwrap_key(&protected)
        .map_err(|_| VaultError::InvalidPassword)?;
    parse_sync(cache.encrypted_vault.as_bytes(), &user_key).map_err(VaultError::Sync)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::auth::ServerConfig;
    use crate::vault::crypto::{Kdf, SymmetricKey};
    use crate::vault::http::{HttpRequest, HttpResponse, HttpTransport, TransportError};
    use std::cell::RefCell;

    /// Serves canned responses in order (prelogin → token → sync).
    struct QueueTransport {
        responses: RefCell<Vec<HttpResponse>>,
    }

    impl HttpTransport for QueueTransport {
        fn send(&self, _request: HttpRequest) -> Result<HttpResponse, TransportError> {
            Ok(self.responses.borrow_mut().remove(0))
        }
    }

    fn ok(body: String) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.into_bytes(),
        }
    }

    const EMAIL: &str = "vault@example.com";
    const PASSWORD: &str = "master-pw";

    /// Build the three server responses for a successful unlock, with the vault
    /// key + ciphers produced by the real crypto so the whole chain is exercised.
    fn unlock_responses() -> Vec<HttpResponse> {
        let kdf = Kdf::Pbkdf2 { iterations: 1000 };
        let master = MasterKey::derive(PASSWORD, EMAIL, kdf).unwrap();
        let stretched = master.stretch().unwrap();

        let mut vault_bytes = [0u8; 64];
        for (i, b) in vault_bytes.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(5).wrapping_add(2);
        }
        let protected_key = stretched.encrypt(&vault_bytes).unwrap().to_string();
        let user_key = SymmetricKey::from_bytes(&vault_bytes).unwrap();
        let e = |s: &str| user_key.encrypt(s.as_bytes()).unwrap().to_string();

        let sync_body = format!(
            r#"{{"ciphers":[{{"id":"c1","type":1,"name":"{name}","login":{{
                "username":"{user}","password":"{pass}","uris":[{{"uri":"{uri}"}}]
            }}}}]}}"#,
            name = e("Example"),
            user = e("alice"),
            pass = e("s3cret"),
            uri = e("https://example.com"),
        );
        vec![
            ok(r#"{"kdf":0,"kdfIterations":1000}"#.to_string()),
            ok(format!(
                r#"{{"access_token":"at","refresh_token":"rt","expires_in":3600,"Key":"{protected_key}"}}"#
            )),
            ok(sync_body),
        ]
    }

    fn manager(
        responses: Vec<HttpResponse>,
        cache_path: Option<String>,
    ) -> VaultManager<QueueTransport> {
        let transport = QueueTransport {
            responses: RefCell::new(responses),
        };
        VaultManager::new(
            VaultClient::new(transport, ServerConfig::bitwarden_cloud()),
            Device::generate("browser"),
            cache_path,
        )
    }

    #[test]
    fn unlock_online_then_lookup_then_lock() {
        let mut m = manager(unlock_responses(), None);
        assert!(m.is_locked());

        m.unlock_online(
            EMAIL.to_string(),
            PASSWORD.to_string(),
            Grant::Password,
            None,
        )
        .unwrap();
        assert!(!m.is_locked());

        // The decrypted login is found by host, and its fields decrypted.
        let hits = m.lookup("example.com", &UsageMap::new()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].username.as_deref(), Some("alice"));
        assert_eq!(hits[0].password.as_deref(), Some("s3cret"));
        // A different site matches nothing.
        assert!(m.lookup("other.org", &UsageMap::new()).unwrap().is_empty());

        m.lock();
        assert!(m.is_locked());
        assert!(matches!(
            m.lookup("example.com", &UsageMap::new()),
            Err(VaultError::Locked)
        ));
    }

    #[test]
    fn lookup_while_locked_errors() {
        let m = manager(vec![], None);
        assert!(matches!(
            m.lookup("example.com", &UsageMap::new()),
            Err(VaultError::Locked)
        ));
    }

    #[test]
    fn offline_unlock_from_cache_roundtrips() {
        let dir = std::env::temp_dir().join("browser-vault-manager-offline");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vault.json").to_str().unwrap().to_string();

        // 1. Online unlock writes the encrypted cache.
        let mut online = manager(unlock_responses(), Some(path.clone()));
        online
            .unlock_online(
                EMAIL.to_string(),
                PASSWORD.to_string(),
                Grant::Password,
                None,
            )
            .unwrap();

        // 2. A fresh manager (no network responses) unlocks purely from cache.
        let mut offline = manager(vec![], Some(path.clone()));
        offline.unlock_offline(PASSWORD).unwrap();
        let hits = offline
            .lookup("accounts.example.com", &UsageMap::new())
            .unwrap();
        assert_eq!(hits.len(), 1, "domain match reaches the cached login");
        assert_eq!(hits[0].password.as_deref(), Some("s3cret"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn offline_unlock_with_wrong_password_is_rejected() {
        let dir = std::env::temp_dir().join("browser-vault-manager-wrongpw");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vault.json").to_str().unwrap().to_string();

        let mut online = manager(unlock_responses(), Some(path.clone()));
        online
            .unlock_online(
                EMAIL.to_string(),
                PASSWORD.to_string(),
                Grant::Password,
                None,
            )
            .unwrap();

        let mut offline = manager(vec![], Some(path.clone()));
        assert!(matches!(
            offline.unlock_offline("not-the-password"),
            Err(VaultError::InvalidPassword)
        ));
        assert!(offline.is_locked());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn offline_unlock_without_cache_is_nocache() {
        let mut m = manager(vec![], Some("/nonexistent/dir/vault.json".to_string()));
        assert!(matches!(
            m.unlock_offline(PASSWORD),
            Err(VaultError::NoCache)
        ));
    }

    #[test]
    fn import_requires_unlock_and_reports_count() {
        let records = [ImportRecord {
            name: "x".to_string(),
            password: Some("p".to_string()),
            ..Default::default()
        }];

        // Locked → refused.
        let mut locked = manager(vec![], None);
        assert!(matches!(locked.import(&records), Err(VaultError::Locked)));

        // Unlocked → merge-import: pre-sync (empty), upload (200), resync (empty).
        // With an empty vault nothing dedupes, so the one record is added.
        let mut responses = unlock_responses();
        responses.push(ok(r#"{"ciphers":[]}"#.to_string())); // merge's initial resync
        responses.push(ok("[]".to_string())); // /ciphers/import
        responses.push(ok(r#"{"ciphers":[]}"#.to_string())); // final resync
        let mut m = manager(responses, None);
        m.unlock_online(
            EMAIL.to_string(),
            PASSWORD.to_string(),
            Grant::Password,
            None,
        )
        .unwrap();
        assert_eq!(m.import(&records).unwrap(), 1);
    }
}
