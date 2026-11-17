//! Per-login last-used timestamps - the recency signal behind [`lookup`].
//!
//! [`lookup`](crate::vault::lookup) ranks by how strongly a login matched the
//! page, but within a tier the order was the vault's own sync order - fixed
//! forever, so the entry you actually use never rose to the top. This module is
//! the missing signal: a login id → "when did the user last sign in with this"
//! map, updated on the explicit fill/copy gestures and consulted when ordering.
//!
//! **Local only, never synced.** Bitwarden's cipher model has no last-used field
//! (`revisionDate` is the *edit* date), so this is ours and stays on this
//! machine, mirroring how Bitwarden's own extension tracks recency client-side.
//!
//! **Plaintext at rest, deliberately** (`root_cache_dir()/vault-usage.json`, next
//! to the equally-plaintext `vault-device`). It holds only opaque cipher UUIDs and
//! unix timestamps - no secret, no site, no username. The strongest inference it
//! supports is "some vault item was used at time T". Encrypting it would mean
//! folding it into [`cache`](crate::vault::cache), whose blob is the verbatim
//! server `/sync` response - rewriting that on every keystroke-cheap fill buys
//! nothing over what it would cost.
//!
//! Private-window use is **not** recorded; that guard lives at the one choke
//! point, [`crate::vault::service::note_used`].

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Login id → unix seconds of last use.
pub type UsageMap = HashMap<String, u64>;

/// The loaded map - `None` until first touched (lazy: a locked vault never reads).
static USAGE: Mutex<Option<UsageMap>> = Mutex::new(None);

fn path() -> String {
    format!("{}/vault-usage.json", crate::root_cache_dir())
}

fn with<R>(f: impl FnOnce(&mut UsageMap) -> R) -> R {
    let mut guard = USAGE.lock().unwrap_or_else(|p| p.into_inner());
    let map = guard.get_or_insert_with(|| load(&path()).unwrap_or_default());
    f(map)
}

/// A copy of the current map, for [`lookup`](crate::vault::lookup) to sort with.
pub fn snapshot() -> UsageMap {
    with(|map| map.clone())
}

/// Stamp login `id` as used *now* and persist. Callers go through
/// [`crate::vault::service::note_used`], which applies the private-window guard.
pub fn note_used(id: &str) {
    if id.is_empty() {
        return;
    }
    let map = with(|map| {
        map.insert(id.to_string(), now_secs());
        map.clone()
    });
    save(&path(), &map);
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Read the map. `None` if missing (never used here) or unparsable - logged and
/// treated as absent, never fatal: the list just falls back to vault order.
fn load(path: &str) -> Option<UsageMap> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(map) => Some(map),
        Err(err) => {
            eprintln!("[vault] failed to parse {path}: {err}");
            None
        }
    }
}

/// Write atomically (temp file + rename), mirroring [`crate::vault::cache`].
/// Failures are logged, never fatal - only the ordering degrades.
fn save(path: &str, map: &UsageMap) {
    let json = match serde_json::to_vec(map) {
        Ok(json) => json,
        Err(err) => {
            eprintln!("[vault] failed to serialize usage: {err}");
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

    #[test]
    fn save_load_roundtrip() {
        let dir = std::env::temp_dir().join("browser-vault-usage-roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vault-usage.json");
        let path = path.to_str().unwrap();

        let map = UsageMap::from([("a".to_string(), 1_700_000_000u64)]);
        save(path, &map);
        assert_eq!(load(path), Some(map));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_or_corrupt_is_none() {
        assert!(load("/nonexistent/dir/vault-usage.json").is_none());

        let dir = std::env::temp_dir().join("browser-vault-usage-corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vault-usage.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(load(path.to_str().unwrap()).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saved_map_carries_no_secret_beyond_id_and_time() {
        // The at-rest guarantee this file's plaintext choice rests on: ids +
        // timestamps only, so the JSON is exactly what the module doc claims.
        let map = UsageMap::from([("cipher-uuid".to_string(), 42u64)]);
        let json = serde_json::to_string(&map).unwrap();
        assert_eq!(json, r#"{"cipher-uuid":42}"#);
    }
}
