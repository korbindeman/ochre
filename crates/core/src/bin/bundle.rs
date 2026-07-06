//! Dev bundler — assemble `Browser.app` so the CEF binary can run.
//!
//! CEF on macOS must run from a `.app` bundle whose `Contents/Frameworks` holds
//! `Chromium Embedded Framework.framework` and one helper app per subprocess
//! role (GPU / Renderer / Plugin / Alerts). This tool builds the `browser` and
//! `browser_helper` binaries (debug) and assembles that structure using
//! cef-rs's bundling utilities.
//!
//! Output: `target/bundle/browser.app`. Run it with `just run`.
//!
//! Note: `cef::build_util::build_bundle` always bundles the *debug* binaries
//! (it targets `target/debug`), so this is a debug-only dev convenience.

use std::path::Path;

use cef::build_util::mac::{build_bundle, BundleInfo};
use semver::Version;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = Path::new("target/bundle");
    std::fs::create_dir_all(output)?;

    let bundle_info = BundleInfo::new(
        "browser",
        "dev.browser.Browser",
        "Browser",
        "English",
        Version::new(0, 1, 0),
    );

    // Builds `browser` + `browser_helper`, then lays out the .app bundle.
    let app = build_bundle(output, "browser", bundle_info)?;
    println!("Bundled: {}", app.display());
    Ok(())
}
