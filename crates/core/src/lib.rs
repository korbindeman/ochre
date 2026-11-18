//! Browser core - CEF engine wrapper and lifecycle.
//!
//! This crate provides the CEF initialization, browser lifecycle management,
//! and the minimal client/handler implementations needed to render a single
//! browser instance in a native macOS window. It is the foundation for the
//! browser's Rust core (engine module per the architecture spec §4).
//!
//! During the Step 0 spike this crate is a near-empty placeholder so the
//! workspace compiles and downstream crates (ffi) have a dependency target.

//#![forbid(unsafe_code)]

/// Crate version surfaced for FFI / diagnostics.
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod adblock;
pub mod app;
pub mod cdp;
pub mod client;
pub mod consent;
pub mod crash_report;
pub mod embed;
pub mod favicon_selftest;
pub mod friction;
pub mod fuzzy;
pub mod history;
pub mod import;
pub mod inject;
pub mod mac;
pub mod native_shell;
pub mod overlay;
pub mod private_window;
pub mod product;
pub mod redirect_selftest;
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

use cef::{
    api_hash, args::Args, execute_process, initialize, run_message_loop, shutdown, sys,
    ImplCommandLine, MainArgs, Settings,
};
use std::path::Path;

/// Initialize CEF and run the message loop.
///
/// This is the main entry point for the browser process. It:
/// 1. Loads the CEF framework (macOS).
/// 2. Initializes the CEF API version hash.
/// 3. Sets up the macOS application (NSApplication subclass implementing CEF protocols).
/// 4. Initializes CEF with a SimpleApp handler.
/// 5. Runs the message loop until shutdown.
pub fn run() -> Result<(), &'static str> {
    crash_report::install();
    let _library = load_cef();

    let args = Args::new();
    let Some(cmd_line) = args.as_cmd_line() else {
        return Err("Failed to parse command line arguments");
    };

    let main_args = args.as_main_args();
    run_browser_process(main_args, &cmd_line);
    Ok(())
}

/// Load the CEF framework library on macOS and verify the API version.
fn load_cef() -> mac::LibraryLoader {
    let loader = mac::load_cef_framework();
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    mac::setup_simple_application();
    loader
}

/// Run the browser process: execute_process check, initialize CEF, run message loop.
fn run_browser_process(main_args: &MainArgs, cmd_line: &cef::CommandLine) {
    let switch = cef::CefString::from("type");
    let is_browser_process = cmd_line.has_switch(Some(&switch)) != 1;

    // Pass `BrowserApp` so helper subprocesses run `on_before_command_line_processing`
    // and pick up `use-mock-keychain` plus the WebAuthn guard
    // (see `app::append_mock_keychain_switch` / `append_webauthn_guard_switch`).
    let mut app = app::BrowserApp::new();
    let ret = execute_process(Some(main_args), Some(&mut app), std::ptr::null_mut());

    if is_browser_process {
        eprintln!("[browser] launch browser process");
        assert_eq!(ret, -1, "cannot execute browser process");
    } else {
        let process_type = cef::CefString::from(&cmd_line.switch_value(Some(&switch)));
        eprintln!("[browser] launch helper process: {process_type}");
        assert!(ret >= 0, "cannot execute helper process");
        return;
    }

    let settings = Settings {
        no_sandbox: !cfg!(feature = "sandbox") as _,
        root_cache_path: cef::CefString::from(root_cache_dir().as_str()),
        ..Default::default()
    };
    assert_eq!(
        initialize(
            Some(main_args),
            Some(&settings),
            Some(&mut app),
            std::ptr::null_mut()
        ),
        1
    );

    // CEF installs its own signal handlers during initialize; reclaim them so
    // local crash dumps still fire for in-process faults.
    crash_report::reinstall_after_cef();

    // Bring the app to the foreground so the CEF window becomes key/visible.
    mac::setup_app_delegate();

    run_message_loop();

    shutdown();
}

/// Application Support folder name for this build profile.
///
/// Debug (`just run`) and release (`just dogfood`) must not share CEF cache,
/// session, history, vault, or any other persisted state - and separate
/// directories also let both run at once without Chromium's process singleton
/// colliding.
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
    std::fs::create_dir_all(&dir).expect("failed to create the CEF root cache directory");
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
