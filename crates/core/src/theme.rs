//! Appearance - forward the light/dark color scheme to web content, with a
//! Light / Dark / System preference (ADR-0030).
//!
//! CEF's ALLOY runtime does not propagate the OS appearance to web content, so
//! `prefers-color-scheme` is stuck at `light` regardless of the Mac's setting.
//! We drive it explicitly through the **global request context** - every browser
//! is created with no request context of its own ([`crate::embed::create_browser`]
//! passes `None`), so they all share the global one, and a single
//! `set_chrome_color_scheme` call themes every tab, live.
//!
//! The core owns the *preference* (persisted) and resolves it to a concrete
//! light/dark scheme. It does **not** detect the OS appearance itself: the host
//! forwards it via [`set_system_dark`] (on launch and on every OS theme change),
//! because that detection is platform-specific and the ALLOY runtime - the very
//! place the propagation is broken - can't be trusted to track the OS for us
//! (ADR-0030). In `System` mode the resolved scheme follows the last forwarded
//! value; in `Light`/`Dark` it is fixed and the OS value is ignored.

use cef::{request_context_get_global_context, ColorVariant, ImplRequestContext};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The user's appearance choice. `System` tracks the OS; the other two override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

impl ThemePreference {
    /// FFI wire value (`browser_theme_*`): 0 System, 1 Light, 2 Dark.
    pub fn to_i32(self) -> i32 {
        match self {
            ThemePreference::System => 0,
            ThemePreference::Light => 1,
            ThemePreference::Dark => 2,
        }
    }

    /// Parse the FFI wire value; anything unexpected falls back to `System`.
    pub fn from_i32(value: i32) -> Self {
        match value {
            1 => ThemePreference::Light,
            2 => ThemePreference::Dark,
            _ => ThemePreference::System,
        }
    }
}

/// Persisted appearance state. A struct (not a bare enum) for forward-compat
/// (per-site override, time-scheduled auto light/dark) without a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct ThemeState {
    preference: ThemePreference,
}

impl Default for ThemeState {
    fn default() -> Self {
        // Track the OS out of the box - the fix's whole point.
        Self {
            preference: ThemePreference::System,
        }
    }
}

static STATE: Mutex<ThemeState> = Mutex::new(ThemeState {
    preference: ThemePreference::System,
});

/// The last OS appearance the host forwarded. Only consulted in `System` mode.
static SYSTEM_DARK: AtomicBool = AtomicBool::new(false);

/// The current appearance preference (for the menu's radio checkmarks).
pub fn preference() -> ThemePreference {
    STATE.lock().unwrap_or_else(|p| p.into_inner()).preference
}

/// Set the appearance preference (persisted) and re-apply it to web content +
/// return so the host can update `NSApp.appearance`. Main thread (touches the
/// CEF request context).
pub fn set_preference(preference: ThemePreference) {
    {
        let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
        if state.preference == preference {
            // No change - still re-apply (harmless) so a redundant menu click
            // is idempotent, but skip the disk write.
            apply();
            return;
        }
        state.preference = preference;
    }
    save();
    apply();
}

/// The host reports the current OS appearance (launch + every OS theme change).
/// Stored unconditionally; re-applied so a live OS flip re-themes open tabs while
/// in `System` mode. Main thread.
pub fn set_system_dark(is_dark: bool) {
    SYSTEM_DARK.store(is_dark, Ordering::Relaxed);
    apply();
}

/// Load the persisted preference into the global. Absent/corrupt ⇒ default
/// (System). Call once at startup, after `cef_initialize`, before the first
/// browser. Does not apply - the host drives the first apply via
/// [`set_system_dark`] once it knows the OS appearance.
pub fn load_persisted() {
    if let Some(state) = load_from(&state_path()) {
        *STATE.lock().unwrap_or_else(|p| p.into_inner()) = state;
    }
}

/// Whether the effective (resolved) scheme is dark, given the preference and the
/// last forwarded OS appearance.
fn resolved_dark() -> bool {
    match preference() {
        ThemePreference::Light => false,
        ThemePreference::Dark => true,
        ThemePreference::System => SYSTEM_DARK.load(Ordering::Relaxed),
    }
}

/// Push the resolved scheme onto the global request context. All browsers share
/// it, so this themes every tab live (no reload). Main thread / UI thread only -
/// request-context operations must run there.
fn apply() {
    let variant = if resolved_dark() {
        ColorVariant::DARK
    } else {
        ColorVariant::LIGHT
    };
    if let Some(context) = request_context_get_global_context() {
        // `user_color` (2nd arg) seeds the generated-palette variants only; for
        // LIGHT/DARK it is ignored, so 0.
        context.set_chrome_color_scheme(variant, 0);
    }
}

fn state_path() -> String {
    format!("{}/theme.json", crate::root_cache_dir())
}

fn load_from(path: &str) -> Option<ThemeState> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(state) => Some(state),
        Err(err) => {
            eprintln!("[theme] failed to parse {path}: {err}");
            None
        }
    }
}

fn save() {
    let snapshot = *STATE.lock().unwrap_or_else(|p| p.into_inner());
    let path = state_path();
    let Ok(json) = serde_json::to_vec_pretty(&snapshot) else {
        return;
    };
    let tmp = format!("{path}.tmp");
    if let Err(err) = std::fs::write(&tmp, &json).and_then(|()| std::fs::rename(&tmp, &path)) {
        eprintln!("[theme] failed to write {path}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_value_round_trips() {
        for pref in [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ] {
            assert_eq!(ThemePreference::from_i32(pref.to_i32()), pref);
        }
        // Out-of-range wire values fall back to System (defensive FFI boundary).
        assert_eq!(ThemePreference::from_i32(99), ThemePreference::System);
        assert_eq!(ThemePreference::from_i32(-1), ThemePreference::System);
    }

    #[test]
    fn state_json_round_trips_and_defaults_to_system() {
        let state = ThemeState {
            preference: ThemePreference::Dark,
        };
        let back: ThemeState =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(state, back);
        assert_eq!(ThemeState::default().preference, ThemePreference::System);
    }

    #[test]
    fn preference_serializes_lowercase() {
        // The persisted form is stable, human-readable JSON.
        let json = serde_json::to_string(&ThemeState {
            preference: ThemePreference::Light,
        })
        .unwrap();
        assert_eq!(json, r#"{"preference":"light"}"#);
    }
}
