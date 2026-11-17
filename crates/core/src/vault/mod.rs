//! Password vault - a Bitwarden-compatible client (spec §10, ADR-0016).
//!
//! The vault talks to bitwarden.com or a self-hosted Vaultwarden and surfaces
//! saved logins for autofill. It is built **core-first, crypto-first**: this
//! module tree is frontend-agnostic Rust; the native UI only ever sees decrypted
//! logins across the FFI boundary.
//!
//! Slices (ADR-0016), in dependency order:
//!   1. [`crypto`]        - the key hierarchy + Bitwarden `EncString`.
//!   2. [`auth`]          - prelogin, token grant, 2FA, device id.
//!   3. [`sync`]          - `GET /api/sync`, decrypt the login-item model.
//!   4. [`lookup`] + [`cache`] + [`manager`] - `vault_lookup(host)`, the
//!      lock/unlock state machine, and the encrypted-at-rest cache. **← here.**
//!   5. FFI + native UI    - `browser_vault_*`, unlock / autofill prompts.
//!
//! Slices 1–4 plus [`import`] exist today - the whole frontend-agnostic core.
//! What remains is shell-gated: the FFI surface (P2-12), the native UI (P2-13),
//! and autofill (P2-14..P2-16). [`http`] is the transport seam shared by [`auth`],
//! [`sync`], and [`import`]; [`manager::VaultManager`] is the entry point.

pub mod auth;
pub mod autofill;
pub mod cache;
pub mod crypto;
pub mod http;
pub mod import;
pub mod lookup;
pub mod manager;
pub mod search;
pub mod service;
pub mod sync;
pub mod usage;
