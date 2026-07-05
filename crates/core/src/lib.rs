//! Browser core: the Rust heart of the browser.
//!
//! Eventually holds the engine, tabs/spaces model, adblock integration,
//! vault client, and CDP surface (architecture spec §17).
//!
//! During the Step 0 spike this crate is a near-empty placeholder so the
//! workspace compiles and downstream crates (ffi) have a dependency target.

#![forbid(unsafe_code)]

/// Crate version surfaced for FFI / diagnostics.
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
