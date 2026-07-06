//! Bundled application resources.
//!
//! CEF loads its own runtime resources (`.pak` files, `icudtl.dat`, locales)
//! from inside `Chromium Embedded Framework.framework/Resources` at load time,
//! so nothing is required here to render a page. This module is the home for
//! app-level embedded resources (custom error/internal pages) per the
//! architecture spec, and currently exists to satisfy the `pub mod resources`
//! declaration in the crate root.
