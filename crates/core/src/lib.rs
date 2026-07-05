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

pub mod app;
pub mod client;
pub mod mac;
pub mod resources;

use cef::{
    args::Args, api_hash, execute_process, initialize, run_message_loop, shutdown, sys,
    MainArgs, Settings,
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
    run_browser_process(&main_args, &cmd_line);
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
        ..Default::default()
    };
    assert_eq!(
        initialize(Some(main_args), Some(&settings), Some(&mut app), std::ptr::null_mut()),
        1
    );

    // Set up the NSApplication delegate (loads MainMenu.nib, sets up app menus).
    let _delegate = mac::setup_app_delegate();

    run_message_loop();

    shutdown();
}
