//! Browser data import - parse foreign browser stores and merge into the tab model.
//!
//! Today: Arc for macOS via `StorableSidebar.json` (spaces, folders, pinned tabs,
//! favorites). The parser is frontend-agnostic; native hosts only supply a path and
//! choose merge vs. replace.

mod apply;
mod arc;

pub use apply::{apply_arc_import, ApplyError, ApplyResult};
pub use arc::{
    parse_arc_sidebar, ArcFolder, ArcImport, ArcImportError, ArcImportPreview, ArcPinnedItem,
    ArcSpace, ArcTab,
};

use std::path::Path;

/// The default Arc sidebar file on macOS (`~/Library/Application Support/Arc/…`).
pub fn default_arc_sidebar_path() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| {
            Path::new(&home)
                .join("Library/Application Support/Arc/StorableSidebar.json")
                .display()
                .to_string()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Parse + preview counts without touching the live tab store.
pub fn preview_arc_import(path: &str) -> Result<ArcImportPreview, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("could not read {path}: {e}"))?;
    let parsed = parse_arc_sidebar(&bytes).map_err(|e| e.to_string())?;
    Ok(parsed.preview())
}

/// Parse `path` and merge the sidebar into the tab model.
pub fn import_arc_sidebar(path: &str, replace: bool) -> Result<ApplyResult, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("could not read {path}: {e}"))?;
    let parsed = parse_arc_sidebar(&bytes).map_err(|e| e.to_string())?;
    apply_arc_import(&parsed, replace).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session;
    use crate::tabs::{self, snapshot};

    fn fixture_path(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("StorableSidebar.json");
        std::fs::write(
            &path,
            include_bytes!("../../tests/fixtures/arc_sidebar_minimal.json"),
        )
        .unwrap();
        path
    }

    /// Parse → merge/replace → snapshot + session persistence. The standing
    /// regression for Arc import; runs against `tests/fixtures/arc_sidebar_minimal.json`.
    #[test]
    fn arc_import_end_to_end() {
        tabs::with_global_store_test_lock(|| {
            let dir = std::env::temp_dir()
                .join(format!("browser-arc-import-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let fixture = fixture_path(&dir);
            let fixture_str = fixture.to_str().unwrap();
            let session_path = dir.join("session.json");
            let session_str = session_path.to_str().unwrap();

            tabs::test_reset_store(session_str);

            // Merge: default Home space survives; Arc spaces append after it.
            let merged = import_arc_sidebar(fixture_str, false).expect("merge import");
            assert_eq!(merged.spaces, 2);
            assert_eq!(merged.folders, 1);
            assert_eq!(merged.tabs, 3);
            assert_eq!(merged.favorites, 1);
            assert_eq!(merged.skipped_duplicates, 0);

            let snap = snapshot();
            assert_eq!(snap.spaces.len(), 3);
            assert!(snap.spaces.iter().any(|s| s.name == "Home"));

            let work = snap.spaces.iter().find(|s| s.name == "Work").unwrap();
            assert_eq!(work.icon, "💻");
            let work_id = work.id;

            assert_eq!(snap.folders.len(), 1);
            assert_eq!(snap.folders[0].name, "Docs");
            assert_eq!(snap.folders[0].space, work_id);
            let folder_id = snap.folders[0].id;

            let work_tabs: Vec<_> = snap.tabs.iter().filter(|t| t.space == work_id).collect();
            assert_eq!(work_tabs.len(), 3);

            let loose = work_tabs
                .iter()
                .find(|t| t.url == "https://example.com/a")
                .unwrap();
            assert!(loose.pinned && !loose.favorite && loose.folder.is_none());
            assert_eq!(loose.title, "Loose Tab");

            let in_folder = work_tabs
                .iter()
                .find(|t| t.url == "https://example.com/b")
                .unwrap();
            assert_eq!(in_folder.folder, Some(folder_id));
            assert!(in_folder.pinned);

            let fav = work_tabs
                .iter()
                .find(|t| t.url == "https://example.com/fav")
                .unwrap();
            assert!(fav.favorite && !fav.pinned);

            // Imported tabs stay suspended until the host hydrates them.
            assert!(snap.tabs.iter().all(|t| !t.hydrated));

            let saved = session::load(session_str).expect("session persisted");
            assert_eq!(saved.spaces.len(), 3);
            assert_eq!(saved.tabs.len(), 3);

            // Second merge appends another copy of each Arc space (duplicate
            // URLs are only skipped within the *same* space, not across spaces).
            let again = import_arc_sidebar(fixture_str, false).expect("re-merge");
            assert_eq!(again.spaces, 2);
            assert_eq!(again.tabs, 3);
            assert_eq!(again.skipped_duplicates, 0);
            assert_eq!(snapshot().spaces.len(), 5); // Home + 2× Work + 2× Personal

            // Replace wipes the session and imports only Arc data.
            let replaced = import_arc_sidebar(fixture_str, true).expect("replace import");
            assert_eq!(replaced.spaces, 2);
            assert_eq!(replaced.tabs, 3);

            let snap = snapshot();
            assert_eq!(snap.spaces.len(), 2);
            assert!(!snap.spaces.iter().any(|s| s.name == "Home"));
            assert_eq!(snap.spaces[0].name, "Work");
            assert_eq!(snap.active_space, snap.spaces[0].id);

            let saved = session::load(session_str).expect("replace persisted");
            assert_eq!(saved.spaces.len(), 2);
            assert_eq!(saved.tabs.len(), 3);

            std::fs::remove_dir_all(&dir).ok();
        });
    }

    /// Manual smoke against a live Arc install: `cargo test -p browser-core real_arc_sidebar -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_arc_sidebar_preview() {
        let Some(path) = default_arc_sidebar_path() else {
            return;
        };
        let preview = preview_arc_import(&path).expect("preview");
        eprintln!(
            "spaces={} folders={} tabs={} favorites={}",
            preview.spaces, preview.folders, preview.tabs, preview.favorites
        );
        assert!(preview.spaces > 0);
        assert!(preview.tabs > 0);
    }
}
