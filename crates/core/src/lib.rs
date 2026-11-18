//! Browser core: the engine wrapper (WebKit, ADR-0072) and every piece of
//! browser logic the native frontends share - tabs, spaces, history, ad
//! blocking, the vault, import, settings.

//#![forbid(unsafe_code)]

/// Crate version surfaced for FFI / diagnostics.
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod adblock;
pub mod consent;
pub mod crash_report;
pub mod embed;
pub mod engine;
pub mod friction;
pub mod fuzzy;
pub mod history;
pub mod import;
pub mod inject;
pub mod mac;
pub mod main_thread;
pub mod omnibox;
pub mod overlay;
pub mod private_window;
pub mod product;
pub mod resources;
pub mod search_engine;
pub mod session;
pub mod settings;
pub mod tab_suspension;
pub mod tabs;
pub mod theme;
pub mod theme_color;
pub mod update;
pub mod vault;

use std::path::Path;

/// Application Support folder name for this build profile.
///
/// Debug (`just run`) and release (`just dogfood`) must not share session,
/// history, vault, or any other persisted state, so both can run at once.
pub(crate) fn profile_data_dir_name() -> &'static str {
    if cfg!(debug_assertions) {
        product::NAME_DEV
    } else {
        product::NAME
    }
}

/// Rename a previous product's Application Support folder when the current
/// one does not exist yet (ADR-0060). No-op if the destination is already there.
fn migrate_legacy_profile_dir(support: &Path, dest_name: &str) {
    let dest = support.join(dest_name);
    if dest.exists() {
        return;
    }
    let suffix = if dest_name == product::NAME_DEV {
        product::NAME_DEV
            .strip_prefix(product::NAME)
            .unwrap_or(" Dev")
    } else {
        ""
    };
    for legacy in product::LEGACY_NAMES {
        let src_name = format!("{legacy}{suffix}");
        if src_name == dest_name {
            continue;
        }
        let src = support.join(&src_name);
        if !src.is_dir() {
            continue;
        }
        match std::fs::rename(&src, &dest) {
            Ok(()) => {
                eprintln!("[profile] migrated {src_name} → {dest_name}");
                return;
            }
            Err(e) => {
                eprintln!("[profile] failed to migrate {src_name} → {dest_name}: {e}");
            }
        }
    }
}

/// Absolute directory CEF uses for its cache and the single-instance lock.
///
/// CEF requires an absolute `root_cache_path`. Without one it falls back to a
/// shared default, and every launch collides on Chromium's process singleton -
/// a second `browser` detects the first, hands its command line off to it, and
/// exits. Pinning a stable per-user directory lets repeated dev launches reuse
/// the same profile instead of racing the singleton.
pub(crate) fn root_cache_dir() -> String {
    let home = std::env::var("HOME").expect("HOME environment variable is not set");
    let support = Path::new(&home).join("Library/Application Support");
    let dest_name = profile_data_dir_name();
    migrate_legacy_profile_dir(&support, dest_name);
    let dir = support.join(dest_name);
    std::fs::create_dir_all(&dir).expect("failed to create the profile directory");
    dir.to_string_lossy().into_owned()
}

#[cfg(test)]
mod profile_tests {
    use super::{migrate_legacy_profile_dir, product, profile_data_dir_name};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn profile_data_dir_matches_build_profile() {
        if cfg!(debug_assertions) {
            assert_eq!(profile_data_dir_name(), product::NAME_DEV);
        } else {
            assert_eq!(profile_data_dir_name(), product::NAME);
        }
    }

    #[test]
    fn migrates_legacy_profile_when_dest_missing() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let support = std::env::temp_dir().join(format!("ochre-profile-{unique}"));
        let legacy = support.join("Browser Dev");
        fs::create_dir_all(legacy.join("crashes")).expect("legacy dir");
        fs::write(legacy.join("marker.txt"), "ok").expect("marker");
        migrate_legacy_profile_dir(&support, product::NAME_DEV);
        let dest = support.join(product::NAME_DEV);
        assert!(dest.join("marker.txt").is_file());
        assert!(!legacy.exists());
        let _ = fs::remove_dir_all(&support);
    }

    #[test]
    fn does_not_clobber_existing_profile() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let support = std::env::temp_dir().join(format!("ochre-profile-keep-{unique}"));
        let dest = support.join(product::NAME_DEV);
        let legacy = support.join("Browser Dev");
        fs::create_dir_all(&dest).expect("dest");
        fs::write(dest.join("keep.txt"), "new").expect("dest marker");
        fs::create_dir_all(&legacy).expect("legacy");
        fs::write(legacy.join("old.txt"), "old").expect("legacy marker");
        migrate_legacy_profile_dir(&support, product::NAME_DEV);
        assert!(dest.join("keep.txt").is_file());
        assert!(legacy.join("old.txt").is_file());
        let _ = fs::remove_dir_all(&support);
    }
}
