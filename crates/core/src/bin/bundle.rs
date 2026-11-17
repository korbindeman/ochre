//! CEF bundler - assemble `browser.app` so the standalone binary can run.
//!
//! CEF on macOS must run from a `.app` bundle whose `Contents/Frameworks` holds
//! `Chromium Embedded Framework.framework` and one helper app per subprocess
//! role (GPU / Renderer / Plugin / Alerts). Builds `browser` + `browser_helper`
//! and lays out that structure via cef-rs's `bundle` helper.
//!
//! Output:
//! - debug (default): `target/bundle/browser.app` - used by `just run` / `just bundle`
//! - release (`--release`): `target/bundle-release/browser.app` - used by `just dogfood`
//!
//! cef-rs's `build_bundle` hardcodes `target/debug`; we call `bundle` directly so
//! release helpers stay matched to release binaries.

use std::env;
use std::io;
use std::path::Path;
use std::process::Command;

use cef::build_util::mac::{bundle, BundleInfo};
use cef::build_util::metadata::get_cargo_metadata;
use semver::Version;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let release = std::env::args().any(|a| a == "--release");
    let profile = if release { "release" } else { "debug" };
    let output = Path::new(if release {
        "target/bundle-release"
    } else {
        "target/bundle"
    });
    std::fs::create_dir_all(output)?;

    // Match apps/macos/project.yml PRODUCT_BUNDLE_IDENTIFIER so standalone
    // bundles and the SwiftUI shell share one identifier family.
    let (bundle_id, display_name) = if release {
        (
            browser_core::product::BUNDLE_ID,
            browser_core::product::NAME,
        )
    } else {
        (
            browser_core::product::BUNDLE_ID_DEV,
            browser_core::product::NAME_DEV,
        )
    };
    let bundle_info = BundleInfo::new(
        "browser",
        bundle_id,
        display_name,
        "English",
        Version::new(0, 1, 0),
    );

    let cargo_metadata = get_cargo_metadata()?;
    let bundle_metadata = cargo_metadata.parse_bundle_metadata("browser")?;
    let target_path = cargo_metadata.target_directory().join(profile);

    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut build = Command::new(&cargo);
    build
        .arg("build")
        .arg("--bin")
        .arg("browser")
        .arg("--bin")
        .arg(&bundle_metadata.helper_name);
    if release {
        build.arg("--release");
    }
    if let Ok(features) = env::var("BROWSER_CARGO_FEATURES") {
        if !features.is_empty() {
            build.arg("--features").arg(features);
        }
    }
    if !build.status()?.success() {
        return Err(io::Error::from(io::ErrorKind::Interrupted).into());
    }

    let app = bundle(
        output,
        &target_path,
        "browser",
        &bundle_metadata.helper_name,
        bundle_metadata.resources_path,
        bundle_info,
    )?;

    // Give each helper a Chromium-style suffix under the main bundle ID.
    let patch_script = Path::new("scripts/patch-helper-bundle-ids.sh");
    if patch_script.exists() {
        let status = Command::new(patch_script).arg(&app).status()?;
        if !status.success() {
            return Err(io::Error::from(io::ErrorKind::Other).into());
        }
    }

    println!("Bundled ({}): {}", profile, app.display());
    Ok(())
}
