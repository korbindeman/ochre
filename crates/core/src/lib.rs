//! Browser core — CEF engine wrapper and lifecycle.
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
pub mod embed;
pub mod fuzzy;
pub mod history;
pub mod inject;
pub mod mac;
pub mod native_shell;
pub mod redirect_selftest;
pub mod resources;
pub mod session;
pub mod settings;
pub mod tabs;
pub mod theme;
pub mod vault;

use cef::{
    api_hash, args::Args, execute_process, initialize, run_message_loop, shutdown, sys,
    ImplCommandLine, MainArgs, Settings,
};

/// Initialize CEF and run the message loop.
///
/// This is the main entry point for the browser process. It:
/// 1. Loads the CEF framework (macOS).
/// 2. Initializes the CEF API version hash.
/// 3. Sets up the macOS application (NSApplication subclass implementing CEF protocols).
/// 4. Initializes CEF with a SimpleApp handler.
/// 5. Runs the message loop until shutdown.
pub fn run() -> Result<(), &'static str> {
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

    let ret = execute_process(Some(main_args), None, std::ptr::null_mut());

    if is_browser_process {
        eprintln!("[browser] launch browser process");
        assert_eq!(ret, -1, "cannot execute browser process");
    } else {
        let process_type = cef::CefString::from(&cmd_line.switch_value(Some(&switch)));
        eprintln!("[browser] launch helper process: {process_type}");
        assert!(ret >= 0, "cannot execute helper process");
        return;
    }

    let mut app = app::BrowserApp::new();

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

    // Bring the app to the foreground so the CEF window becomes key/visible.
    mac::setup_app_delegate();

    run_message_loop();

    shutdown();
}

/// Application Support folder name for this build profile.
///
/// Debug (`just run`) and release (`just dogfood`) must not share CEF cache,
/// session, history, vault, or any other persisted state — and separate
/// directories also let both run at once without Chromium's process singleton
/// colliding.
pub(crate) fn profile_data_dir_name() -> &'static str {
    if cfg!(debug_assertions) {
        "Browser Dev"
    } else {
        "Browser"
    }
}

/// Absolute directory CEF uses for its cache and the single-instance lock.
///
/// CEF requires an absolute `root_cache_path`. Without one it falls back to a
/// shared default, and every launch collides on Chromium's process singleton —
/// a second `browser` detects the first, hands its command line off to it, and
/// exits. Pinning a stable per-user directory lets repeated dev launches reuse
/// the same profile instead of racing the singleton.
pub(crate) fn root_cache_dir() -> String {
    let home = std::env::var("HOME").expect("HOME environment variable is not set");
    let dir = format!(
        "{home}/Library/Application Support/{}",
        profile_data_dir_name()
    );
    std::fs::create_dir_all(&dir).expect("failed to create the CEF root cache directory");
    dir
}

#[cfg(test)]
mod profile_tests {
    use super::profile_data_dir_name;

    #[test]
    fn profile_data_dir_matches_build_profile() {
        if cfg!(debug_assertions) {
            assert_eq!(profile_data_dir_name(), "Browser Dev");
        } else {
            assert_eq!(profile_data_dir_name(), "Browser");
        }
    }
}
