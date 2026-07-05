//! CEF helper process entry point.
//!
//! CEF spawns helper processes (renderer, GPU, utility) for sandboxing.
//! This binary is the target for those spawns — it re-enters the same
//! `browser_core::run()` entry, which detects the helper role via the
//! `--type` switch and dispatches accordingly.

fn main() {
    browser_core::run().expect("Failed to run browser helper");
}
