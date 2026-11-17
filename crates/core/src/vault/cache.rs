//! Encrypted-at-rest vault cache (ADR-0016).
//!
//! One JSON document (`root_cache_dir()/vault.json`) lets the browser unlock and
//! read the vault **offline** and without a network round-trip at launch. It is
//! safe at rest **by construction** - it holds only:
//!
//! - the account email + [`Kdf`] (public parameters),
//! - the **protected** user key (an `EncString`, decryptable only with the master
//!   password via the stretched master key), and
//! - the raw `/sync` blob, whose every field is itself an `EncString` under the
//!   user key.
//!
//! So nothing here is readable without the master password. **Tokens are
//! deliberately not cached** - a bearer/refresh token is a plaintext secret and
//! belongs in OS secure storage (macOS Keychain), wired in the platform/FFI slice
//! (P2-12); until then a relaunch unlocks from cache (offline) and re-authenticates
//! to sync. Writes are atomic (temp file + rename), mirroring [`crate::session`].

use crate::vault::crypto::Kdf;
use serde::{Deserialize, Serialize};

/// The persisted, encrypted-at-rest vault snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultCache {
    pub email: String,
    pub kdf: Kdf,
    /// The wrapped user key (`EncString`), exactly as the server issued it.
    pub protected_key: String,
    /// The raw `/sync` response (JSON of `EncString` fields).
    pub encrypted_vault: String,
}

/// Read the cached vault. `None` if missing (never unlocked here) or unparsable
/// (logged and treated as absent, never fatal).
pub fn load(path: &str) -> Option<VaultCache> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(cache) => Some(cache),
        Err(err) => {
            eprintln!("[vault] failed to parse {path}: {err}");
            None
        }
    }
}

/// Write the cache atomically (temp file + rename). Failures are logged, never
/// fatal - only offline availability degrades.
pub fn save(path: &str, cache: &VaultCache) {
    let json = match serde_json::to_vec(cache) {
        Ok(json) => json,
        Err(err) => {
            eprintln!("[vault] failed to serialize cache: {err}");
            return;
        }
    };
    let tmp = format!("{path}.tmp");
    if let Err(err) = std::fs::write(&tmp, &json).and_then(|()| std::fs::rename(&tmp, path)) {
        eprintln!("[vault] failed to write {path}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> VaultCache {
        VaultCache {
            email: "user@example.com".to_string(),
            kdf: Kdf::Pbkdf2 {
                iterations: 600_000,
            },
            protected_key: "2.aaa|bbb|ccc".to_string(),
            encrypted_vault: r#"{"ciphers":[]}"#.to_string(),
        }
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = std::env::temp_dir().join("browser-vault-cache-roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vault.json");
        let path = path.to_str().unwrap();

        save(path, &sample());
        assert_eq!(load(path), Some(sample()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_is_none() {
        assert!(load("/nonexistent/dir/vault.json").is_none());
    }

    #[test]
    fn load_corrupt_is_none() {
        let dir = std::env::temp_dir().join("browser-vault-cache-corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vault.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(load(path.to_str().unwrap()).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
