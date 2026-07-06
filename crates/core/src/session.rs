//! Session persistence — the tab/space/folder tree across launches (ADR-0013).
//!
//! One JSON document (`root_cache_dir()/session.json`) mirrors the structural
//! state of the tab model: every space, folder, and tab (with its URL, title,
//! pinned flag, and folder membership) plus the active pointers. The tab store
//! writes it through on every structural / URL / title change via an atomic
//! temp-file + rename, so the file is always a complete, current session and a
//! crash never leaves a torn one — there is no fragile "save on quit" step.
//! Read once at startup ([`crate::tabs::init`]); all restored tabs come back
//! *suspended* (no CEF browser) and hydrate lazily when first shown.
//!
//! Transient per-tab state (loading, favicon) is not persisted; page history
//! and favicons live in the SQLite store (`crate::history`).

use serde::{Deserialize, Serialize};

use crate::tabs::{FolderId, SpaceId, TabId};

/// The persisted session — a plain mirror of the tab store's structure. Ids are
/// carried as-is (they are core-assigned, not CEF's), so the restored tree keeps
/// stable identities and the counters resume past the highest saved id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedState {
    pub active_space: SpaceId,
    pub spaces: Vec<SavedSpace>,
    pub folders: Vec<SavedFolder>,
    pub tabs: Vec<SavedTab>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSpace {
    pub id: SpaceId,
    pub name: String,
    pub icon: String,
    /// The space's remembered active tab, restored so switching back lands
    /// where the user left off.
    pub active: Option<TabId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedFolder {
    pub id: FolderId,
    pub space: SpaceId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedTab {
    pub id: TabId,
    pub space: SpaceId,
    pub folder: Option<FolderId>,
    pub pinned: bool,
    pub url: String,
    /// Saved so the restored sidebar shows real titles before hydration.
    pub title: String,
}

/// Read the persisted session. `None` when the file is missing (first launch)
/// or unparsable — a corrupt file is logged and treated as no session rather
/// than aborting; the tab store then starts fresh.
pub fn load(path: &str) -> Option<SavedState> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(state) => Some(state),
        Err(err) => {
            eprintln!("[session] failed to parse {path}: {err}");
            None
        }
    }
}

/// Write the session atomically (temp file + rename, so readers and crashes
/// only ever see a complete document). Failures are logged, never fatal — the
/// browser keeps running; only persistence degrades.
pub fn save(path: &str, state: &SavedState) {
    let json = match serde_json::to_vec(state) {
        Ok(json) => json,
        Err(err) => {
            eprintln!("[session] failed to serialize session: {err}");
            return;
        }
    };
    let tmp = format!("{path}.tmp");
    if let Err(err) = std::fs::write(&tmp, &json).and_then(|()| std::fs::rename(&tmp, path)) {
        eprintln!("[session] failed to write {path}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SavedState {
        SavedState {
            active_space: 1,
            spaces: vec![SavedSpace {
                id: 1,
                name: "Home".into(),
                icon: "🏠".into(),
                active: Some(7),
            }],
            folders: vec![SavedFolder {
                id: 2,
                space: 1,
                name: "Work".into(),
            }],
            tabs: vec![SavedTab {
                id: 7,
                space: 1,
                folder: Some(2),
                pinned: true,
                url: "https://example.com/".into(),
                title: "Example".into(),
            }],
        }
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = std::env::temp_dir().join("browser-session-test-roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");
        let path = path.to_str().unwrap();

        let state = sample();
        save(path, &state);
        let loaded = load(path).expect("saved session loads");
        assert_eq!(loaded.active_space, 1);
        assert_eq!(loaded.spaces[0].active, Some(7));
        assert_eq!(loaded.tabs[0].url, "https://example.com/");
        assert_eq!(loaded.tabs[0].folder, Some(2));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_file_is_none() {
        assert!(load("/nonexistent/dir/session.json").is_none());
    }

    #[test]
    fn load_corrupt_file_is_none() {
        let dir = std::env::temp_dir().join("browser-session-test-corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(load(path.to_str().unwrap()).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
