//! Browser-wide user preferences — persisted under `root_cache_dir()` and
//! owned by the core (ADR-0010: native settings UI is view-only over FFI).

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// When the vault should auto-lock. Timed idle lock is intentionally absent —
/// policies are event-based (quit, relaunch, password use).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultAutoLock {
    /// No automatic lock during a session — manual Lock Now only.
    Never,
    /// Lock when Browser quits gracefully.
    OnBrowserClose,
    /// Require unlock on each relaunch (always locked at startup; also locks on
    /// quit). The secure default — the vault never stays unlocked across sessions.
    OnRestart,
    /// Lock immediately after each action that retrieves a password (fill, copy,
    /// reveal).
    OnPasswordUse,
}

impl VaultAutoLock {
    /// FFI wire value: 0 Never, 1 OnBrowserClose, 2 OnRestart, 3 OnPasswordUse.
    pub fn to_i32(self) -> i32 {
        match self {
            VaultAutoLock::Never => 0,
            VaultAutoLock::OnBrowserClose => 1,
            VaultAutoLock::OnRestart => 2,
            VaultAutoLock::OnPasswordUse => 3,
        }
    }

    pub fn from_i32(value: i32) -> Self {
        match value {
            1 => VaultAutoLock::OnBrowserClose,
            3 => VaultAutoLock::OnPasswordUse,
            0 => VaultAutoLock::Never,
            _ => VaultAutoLock::OnRestart,
        }
    }

    /// Whether to lock the vault on graceful browser shutdown.
    pub fn locks_on_shutdown(self) -> bool {
        matches!(
            self,
            VaultAutoLock::OnBrowserClose | VaultAutoLock::OnRestart | VaultAutoLock::OnPasswordUse
        )
    }

    /// Whether to lock immediately after a password crosses the FFI boundary.
    pub fn locks_after_password_use(self) -> bool {
        self == VaultAutoLock::OnPasswordUse
    }
}

impl Default for VaultAutoLock {
    fn default() -> Self {
        VaultAutoLock::OnRestart
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct SettingsState {
    #[serde(default)]
    vault_auto_lock: VaultAutoLock,
    /// Legacy timed setting — read for migration only.
    #[serde(default, skip_serializing)]
    vault_lock_timeout_secs: Option<u64>,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            vault_auto_lock: VaultAutoLock::OnRestart,
            vault_lock_timeout_secs: None,
        }
    }
}

static STATE: Mutex<SettingsState> = Mutex::new(SettingsState {
    vault_auto_lock: VaultAutoLock::OnRestart,
    vault_lock_timeout_secs: None,
});

/// Load persisted settings at startup (before the first browser). Missing/corrupt
/// ⇒ defaults.
pub fn load_persisted() {
    if let Some(mut state) = load_from(&state_path()) {
        migrate_timed_auto_lock(&mut state);
        *STATE.lock().unwrap_or_else(|p| p.into_inner()) = state;
    }
}

pub fn vault_auto_lock() -> VaultAutoLock {
    STATE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .vault_auto_lock
}

pub fn set_vault_auto_lock(policy: VaultAutoLock) {
    {
        let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
        if state.vault_auto_lock == policy {
            return;
        }
        state.vault_auto_lock = policy;
    }
    save();
}

/// Lock after a password crossed the boundary, when the policy requires it.
pub fn lock_after_password_use_if_needed(lock: impl FnOnce()) {
    if vault_auto_lock().locks_after_password_use() {
        lock();
    }
}

/// Lock on graceful shutdown when the policy requires it.
pub fn lock_on_shutdown_if_needed(lock: impl FnOnce()) {
    if vault_auto_lock().locks_on_shutdown() {
        lock();
    }
}

/// Map a persisted timed auto-lock (removed) to an event-based policy.
fn migrate_timed_auto_lock(state: &mut SettingsState) {
    if let Some(secs) = state.vault_lock_timeout_secs.take() {
        state.vault_auto_lock = if secs == 0 {
            VaultAutoLock::Never
        } else {
            // Any idle timeout maps to the secure session default.
            VaultAutoLock::OnRestart
        };
    }
}

fn state_path() -> String {
    format!("{}/settings.json", crate::root_cache_dir())
}

fn load_from(path: &str) -> Option<SettingsState> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(state) => Some(state),
        Err(err) => {
            eprintln!("[settings] failed to parse {path}: {err}");
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
        eprintln!("[settings] failed to write {path}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_value_round_trips() {
        for policy in [
            VaultAutoLock::Never,
            VaultAutoLock::OnBrowserClose,
            VaultAutoLock::OnRestart,
            VaultAutoLock::OnPasswordUse,
        ] {
            assert_eq!(VaultAutoLock::from_i32(policy.to_i32()), policy);
        }
        assert_eq!(VaultAutoLock::from_i32(99), VaultAutoLock::OnRestart);
    }

    #[test]
    fn defaults_to_on_restart() {
        assert_eq!(
            SettingsState::default().vault_auto_lock,
            VaultAutoLock::OnRestart
        );
    }

    #[test]
    fn json_round_trips() {
        let state = SettingsState {
            vault_auto_lock: VaultAutoLock::OnPasswordUse,
            vault_lock_timeout_secs: None,
        };
        let back: SettingsState =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(state, back);
    }

    #[test]
    fn timed_setting_migrates_to_event_policy() {
        let mut state = SettingsState {
            vault_auto_lock: VaultAutoLock::OnRestart,
            vault_lock_timeout_secs: Some(300),
        };
        migrate_timed_auto_lock(&mut state);
        assert_eq!(state.vault_auto_lock, VaultAutoLock::OnRestart);
        assert!(state.vault_lock_timeout_secs.is_none());

        let mut never = SettingsState {
            vault_auto_lock: VaultAutoLock::OnRestart,
            vault_lock_timeout_secs: Some(0),
        };
        migrate_timed_auto_lock(&mut never);
        assert_eq!(never.vault_auto_lock, VaultAutoLock::Never);
    }

    #[test]
    fn policy_flags() {
        assert!(!VaultAutoLock::Never.locks_on_shutdown());
        assert!(VaultAutoLock::OnBrowserClose.locks_on_shutdown());
        assert!(VaultAutoLock::OnRestart.locks_on_shutdown());
        assert!(VaultAutoLock::OnPasswordUse.locks_on_shutdown());
        assert!(VaultAutoLock::OnPasswordUse.locks_after_password_use());
        assert!(!VaultAutoLock::Never.locks_after_password_use());
    }
}
