//! FFI bridge between Rust core and native UI layers.
//!
//! Will expose core commands + the tab-event stream to Swift (via swift-bridge /
//! UniFFI) and to C# (only if the C# fallback is used) — architecture spec §18.
//!
//! During the Step 0 spike this is a thin placeholder so the workspace builds.

#![forbid(unsafe_code)]

/// Re-export core so FFI consumers reach it through a single crate.
pub use browser_core;

/// Surface the core version to native callers.
pub fn core_version() -> &'static str {
    browser_core::CORE_VERSION
}
