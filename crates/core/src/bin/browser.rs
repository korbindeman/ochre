//! Browser process entry point.
//!
//! This is the main CEF browser process binary. It loads the CEF framework,
//! initializes CEF, creates a browser window, and runs the message loop.
//!
//! Pass `--url=<URL>` to control the initial page (default: Google).
//! Pass `--use-alloy-style` to use the Alloy runtime style.
//! Pass `--use-native` to use the Views framework (default: native popup).
//! Pass `--native-shell` for the pure-Rust AppKit window hosting the embedded
//! browser (the "Option B" hosting model, without the SwiftUI/FFI layer).

fn main() {
    // Helper subprocesses never pass this flag — they run the separate
    // `browser_helper` binary — so this branch only affects the main process.
    if std::env::args().any(|arg| arg == "--native-shell") {
        browser_core::native_shell::run().expect("Failed to run native shell");
        return;
    }
    browser_core::run().expect("Failed to run browser");
}
