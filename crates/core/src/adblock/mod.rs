//! Native request-path ad-blocking (ADR-0020, spec §11).
//!
//! Brave's `adblock` engine (adblock-rust) checks every CEF resource request and
//! cancels the ad/tracker ones. This module owns the process-global engine plus
//! the persisted user state (global switch + per-site allowlist) and the per-tab
//! blocked counters that feed the sidebar shield. The engine wrapper is
//! [`engine`]; list sourcing is [`lists`].
//!
//! **Threading.** [`check`] runs on the CEF **IO thread** (the resource-request
//! handler, [`crate::client`]), and [`lists`] swaps a freshly-fetched engine in
//! from a background thread — so the engine lives behind `RwLock<Arc<Engine>>`
//! (`Engine: Send + Sync`, which is why the crate's `single-thread` feature is
//! off) and the counters are behind a `Mutex`. `check` clones the `Arc` and
//! releases the read lock before matching, so a hot-swap never waits on a match.

pub mod cosmetic;
mod engine;
pub mod lists;

use adblock::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// The live engine, hot-swappable when fresh lists arrive.
static ENGINE: OnceLock<RwLock<Arc<Engine>>> = OnceLock::new();

/// Persisted user state: the global switch + the per-site allowlist.
static STATE: Mutex<AdblockState> = Mutex::new(AdblockState::new());

/// Per-tab count of requests blocked since the tab's last main-frame
/// navigation, keyed by CEF browser identifier. Feeds the sidebar shield.
static COUNTS: OnceLock<Mutex<HashMap<i32, u64>>> = OnceLock::new();

/// Persisted `adblock.json` — the global switch and the allowlist of
/// registrable domains the user has turned blocking **off** for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AdblockState {
    enabled: bool,
    allowlist: Vec<String>,
}

impl AdblockState {
    /// Default: blocking on, nothing allowlisted (the product default for an
    /// ad-blocking browser). `const` so it can seed the `static`.
    const fn new() -> Self {
        Self {
            enabled: true,
            allowlist: Vec::new(),
        }
    }

    fn is_allowlisted(&self, key: &str) -> bool {
        self.allowlist.iter().any(|h| h == key)
    }

    /// Add/remove a registrable domain; returns `true` if the set changed.
    fn set_allowlisted(&mut self, key: String, allowlisted: bool) -> bool {
        match (allowlisted, self.allowlist.iter().position(|h| *h == key)) {
            (true, None) => {
                self.allowlist.push(key);
                true
            }
            (false, Some(i)) => {
                self.allowlist.remove(i);
                true
            }
            _ => false,
        }
    }
}

/// The registrable domain (eTLD+1) the allowlist keys on — lowercased so "turn
/// off for this site" covers every subdomain. Falls back to the raw host when
/// there's no registrable domain (IP literals, single-label intranet hosts).
fn allowlist_key(host: &str) -> String {
    let host = host.trim().to_ascii_lowercase();
    match psl::domain(host.as_bytes()) {
        Some(domain) => String::from_utf8_lossy(domain.as_bytes()).into_owned(),
        None => host,
    }
}

fn state() -> std::sync::MutexGuard<'static, AdblockState> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

fn counts() -> &'static Mutex<HashMap<i32, u64>> {
    COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

// ─── Lifecycle ───────────────────────────────────────────────────────────────

/// Initialize the blocker. Call once at startup, before CEF creates browsers
/// (from [`crate::embed::initialize`]). Loads persisted state, installs the
/// synchronous seed engine (instant, offline), then kicks off the background
/// upgrade to the full cached/upstream lists.
pub fn init() {
    load_persisted();
    install_engine(lists::SEED);
    lists::load_and_refresh();
}

/// Debug-only: install a stable engine carrying the seed **plus** `extra_rules`,
/// and skip the background list refresh (which would hot-swap the test rules
/// away), so the P2-21 cosmetic smoke is deterministic. Triggered by
/// `BROWSER_ADBLOCK_DEMO` in [`crate::embed::initialize`].
#[cfg(debug_assertions)]
pub fn init_demo(extra_rules: &str) {
    load_persisted();
    install_engine(&format!("{}\n{}", lists::SEED, extra_rules));
    eprintln!("[adblock] demo engine installed (seed + test rules, no refresh)");
}

/// Build an engine from filter-list text and install it — setting it on first
/// call (startup) or hot-swapping it (a background list refresh) thereafter.
fn install_engine(rules: &str) {
    let engine = Arc::new(engine::build(rules));
    match ENGINE.get() {
        Some(lock) => *lock.write().unwrap_or_else(|p| p.into_inner()) = engine,
        None => {
            let _ = ENGINE.set(RwLock::new(engine));
        }
    }
}

// ─── The request-path decision (IO thread) ──────────────────────────────────

/// Should this resource request be blocked? Gated by the global switch and the
/// first-party site's allowlist state, then by the engine's network rules.
///
/// `first_party_host` is the top-level page's host (what "turn off for this
/// site" allowlists); `request_type` is the lowercase adblock resource kind
/// (`"script"`, `"image"`, `"sub_frame"`, …) — the CEF mapping lives in the
/// resource-request handler. Runs on the CEF IO thread.
pub fn check(url: &str, source_url: &str, first_party_host: &str, request_type: &str) -> bool {
    {
        let state = state();
        if !state.enabled {
            return false;
        }
        if state.is_allowlisted(&allowlist_key(first_party_host)) {
            return false;
        }
    }
    let Some(lock) = ENGINE.get() else {
        return false;
    };
    // Clone the Arc and drop the read lock before matching, so a concurrent
    // hot-swap never blocks on a match completing.
    let engine = lock.read().unwrap_or_else(|p| p.into_inner()).clone();
    engine::should_block(&engine, url, source_url, request_type)
}

/// The cosmetic-filter stylesheet for page `url`, or `None` when blocking is off,
/// the site is allowlisted, or there are no rules. The document-start injection
/// substrate ([`crate::adblock::cosmetic`]) injects the returned CSS as a
/// `<style>` — element-hiding without a flash (ADR-0020, P2-21).
///
/// Runs on the CEF UI thread (the substrate handler), not the IO thread, but
/// shares the same gate as [`check`]: global switch, then the first-party site's
/// allowlist. Clones the engine `Arc` and drops the read lock before querying.
pub fn cosmetic_css(url: &str) -> Option<String> {
    let host = crate::vault::lookup::host_from_uri(url).unwrap_or_default();
    {
        let state = state();
        if !state.enabled {
            return None;
        }
        if state.is_allowlisted(&allowlist_key(&host)) {
            return None;
        }
    }
    let engine = ENGINE
        .get()?
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let css = engine::cosmetic_css(&engine, url);
    (!css.is_empty()).then_some(css)
}

// ─── Per-tab blocked counts (shield) ─────────────────────────────────────────

/// Record a blocked request against a tab (its CEF browser id).
pub fn note_blocked(tab_id: i32) {
    *counts()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(tab_id)
        .or_insert(0) += 1;
}

/// Blocked count for a tab since its last main-frame navigation.
pub fn blocked_count(tab_id: i32) -> u64 {
    counts()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&tab_id)
        .copied()
        .unwrap_or(0)
}

/// Reset a tab's count — call on a main-frame navigation start (new page).
pub fn reset_count(tab_id: i32) {
    counts()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(tab_id, 0);
}

/// Drop a tab's counter when the tab closes, so the map doesn't grow unbounded.
pub fn forget_tab(tab_id: i32) {
    counts()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&tab_id);
}

// ─── User state (FFI-facing) ─────────────────────────────────────────────────

/// Is blocking globally enabled?
pub fn is_enabled() -> bool {
    state().enabled
}

/// Toggle blocking globally (persisted).
pub fn set_enabled(enabled: bool) {
    {
        let mut state = state();
        if state.enabled == enabled {
            return;
        }
        state.enabled = enabled;
    }
    save();
}

/// Is blocking turned **off** for this host's site (registrable domain)?
pub fn is_site_allowlisted(host: &str) -> bool {
    state().is_allowlisted(&allowlist_key(host))
}

/// Turn blocking off (`true`) or back on (`false`) for this host's site
/// (persisted). Idempotent.
pub fn set_site_allowlisted(host: &str, allowlisted: bool) {
    let changed = state().set_allowlisted(allowlist_key(host), allowlisted);
    if changed {
        save();
    }
}

// ─── Persistence (mirrors `cdp.rs`) ──────────────────────────────────────────

fn state_path() -> String {
    format!("{}/adblock.json", crate::root_cache_dir())
}

/// Load persisted state into the global. Absent/corrupt ⇒ defaults (on).
pub fn load_persisted() {
    if let Some(loaded) = load_from(&state_path()) {
        *state() = loaded;
    }
}

fn save() {
    let snapshot = state().clone();
    save_to(&state_path(), &snapshot);
}

fn load_from(path: &str) -> Option<AdblockState> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(state) => Some(state),
        Err(err) => {
            eprintln!("[adblock] failed to parse {path}: {err}");
            None
        }
    }
}

fn save_to(path: &str, state: &AdblockState) {
    let json = match serde_json::to_vec_pretty(state) {
        Ok(json) => json,
        Err(err) => {
            eprintln!("[adblock] failed to serialize state: {err}");
            return;
        }
    };
    let tmp = format!("{path}.tmp");
    if let Err(err) = std::fs::write(&tmp, &json).and_then(|()| std::fs::rename(&tmp, path)) {
        eprintln!("[adblock] failed to write {path}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_key_is_the_registrable_domain() {
        assert_eq!(allowlist_key("www.example.com"), "example.com");
        assert_eq!(allowlist_key("accounts.google.com"), "google.com");
        // Public-suffix correctness: never collapses across a multi-label eTLD.
        assert_eq!(allowlist_key("a.b.co.uk"), "b.co.uk");
        // Case-insensitive.
        assert_eq!(allowlist_key("News.EXAMPLE.com"), "example.com");
    }

    #[test]
    fn allowlist_add_remove_is_idempotent() {
        let mut s = AdblockState::new();
        assert!(s.set_allowlisted("example.com".into(), true));
        assert!(!s.set_allowlisted("example.com".into(), true)); // no-op re-add
        assert!(s.is_allowlisted("example.com"));
        assert!(s.set_allowlisted("example.com".into(), false));
        assert!(!s.set_allowlisted("example.com".into(), false)); // no-op re-remove
        assert!(!s.is_allowlisted("example.com"));
    }

    #[test]
    fn state_json_round_trips() {
        let state = AdblockState {
            enabled: false,
            allowlist: vec!["example.com".into(), "b.co.uk".into()],
        };
        let json = serde_json::to_vec(&state).unwrap();
        let back: AdblockState = serde_json::from_slice(&json).unwrap();
        assert_eq!(state, back);
    }

    #[test]
    fn default_state_blocks_everything_on() {
        let s = AdblockState::new();
        assert!(s.enabled);
        assert!(s.allowlist.is_empty());
    }

    #[test]
    fn per_tab_counts_track_and_reset() {
        // Distinct high tab ids avoid collision with other (parallel) tests
        // sharing the process-global COUNTS map.
        let tab = 987_001;
        reset_count(tab);
        assert_eq!(blocked_count(tab), 0);
        note_blocked(tab);
        note_blocked(tab);
        assert_eq!(blocked_count(tab), 2);
        reset_count(tab);
        assert_eq!(blocked_count(tab), 0);
        note_blocked(tab);
        forget_tab(tab);
        assert_eq!(blocked_count(tab), 0);
    }
}
