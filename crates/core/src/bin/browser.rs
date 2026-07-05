//! Browser process entry point.
//!
//! This is the main CEF browser process binary. It loads the CEF framework,
//! initializes CEF, creates a browser window, and runs the message loop.
//!
//! Pass `--url=<URL>` to control the initial page (default: Google).
//! Pass `--use-alloy-style` to use the Alloy runtime style.
//! Pass `--use-native` to use the Views framework (default: native popup).

fn main() {
    browser_core::run().expect("Failed to run browser");
}
