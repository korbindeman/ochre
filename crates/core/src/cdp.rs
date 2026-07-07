//! CDP automation surface — loopback-bound, consent-gated (P0-4, spec §13/§3.5).
//!
//! Chromium exposes the full Chrome DevTools Protocol when launched with
//! `--remote-debugging-port`. That is a remote-control surface (anyone who can
//! reach the port drives the browser), so here it is:
//!
//! - **Off by default** — the user must opt in ([`set_enabled`]); the choice is
//!   persisted (`root_cache_dir()/cdp.json`) and read once at startup.
//! - **Loopback only** — bound to `127.0.0.1`, never a routable interface.
//! - **Launch-time** — the port is a Chromium *startup* switch, not runtime
//!   togglable, so a change takes effect on the **next launch**. [`is_enabled`]
//!   reports the persisted choice (what the next launch will do).
//!
//! The switch is applied in the browser process's `on_before_command_line_processing`
//! (see [`crate::embed`]); this module owns the consent state + switch list.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// Chromium's conventional remote-debugging port.
const DEFAULT_PORT: u16 = 9222;

/// The persisted CDP consent state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CdpConfig {
    pub enabled: bool,
    pub port: u16,
}

impl Default for CdpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
        }
    }
}

static CONFIG: Mutex<CdpConfig> = Mutex::new(CdpConfig {
    enabled: false,
    port: DEFAULT_PORT,
});

/// Read the persisted consent into the global. Call once at startup, **before**
/// CEF init (the switch is applied during init). Absent/corrupt file ⇒ defaults
/// (disabled).
pub fn load_persisted() {
    if let Some(config) = load_from(&config_path()) {
        *CONFIG.lock().unwrap_or_else(|p| p.into_inner()) = config;
    }
}

/// The persisted consent (what the next launch will do).
pub fn is_enabled() -> bool {
    config().enabled
}

/// A snapshot of the current config.
pub fn config() -> CdpConfig {
    *CONFIG.lock().unwrap_or_else(|p| p.into_inner())
}

/// Opt in / out (persisted). Takes effect on the next launch.
pub fn set_enabled(enabled: bool) {
    let config = {
        let mut guard = CONFIG.lock().unwrap_or_else(|p| p.into_inner());
        guard.enabled = enabled;
        *guard
    };
    save_to(&config_path(), &config);
}

/// The Chromium switches for `config` (empty when disabled). Loopback-bound.
/// Pure — the CEF command-line handler just applies whatever this returns.
pub fn switches(config: &CdpConfig) -> Vec<(&'static str, String)> {
    if !config.enabled {
        return Vec::new();
    }
    vec![
        ("remote-debugging-port", config.port.to_string()),
        ("remote-debugging-address", "127.0.0.1".to_string()),
    ]
}

fn config_path() -> String {
    format!("{}/cdp.json", crate::root_cache_dir())
}

fn load_from(path: &str) -> Option<CdpConfig> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(config) => Some(config),
        Err(err) => {
            eprintln!("[cdp] failed to parse {path}: {err}");
            None
        }
    }
}

fn save_to(path: &str, config: &CdpConfig) {
    let json = match serde_json::to_vec(config) {
        Ok(json) => json,
        Err(err) => {
            eprintln!("[cdp] failed to serialize config: {err}");
            return;
        }
    };
    let tmp = format!("{path}.tmp");
    if let Err(err) = std::fs::write(&tmp, &json).and_then(|()| std::fs::rename(&tmp, path)) {
        eprintln!("[cdp] failed to write {path}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_config_yields_no_switches() {
        assert!(switches(&CdpConfig {
            enabled: false,
            port: 9222
        })
        .is_empty());
        assert!(switches(&CdpConfig::default()).is_empty());
    }

    #[test]
    fn enabled_config_binds_the_port_to_loopback() {
        let switches = switches(&CdpConfig {
            enabled: true,
            port: 9333,
        });
        assert_eq!(
            switches,
            vec![
                ("remote-debugging-port", "9333".to_string()),
                ("remote-debugging-address", "127.0.0.1".to_string()),
            ]
        );
    }

    #[test]
    fn config_roundtrips_and_missing_is_none() {
        let dir = std::env::temp_dir().join("browser-cdp-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cdp.json");
        let path = path.to_str().unwrap();

        assert_eq!(load_from("/nonexistent/dir/cdp.json"), None);
        let config = CdpConfig {
            enabled: true,
            port: 9500,
        };
        save_to(path, &config);
        assert_eq!(load_from(path), Some(config));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn default_is_disabled() {
        assert!(!CdpConfig::default().enabled);
        assert_eq!(CdpConfig::default().port, DEFAULT_PORT);
    }
}
