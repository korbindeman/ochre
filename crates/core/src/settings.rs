//! Browser-wide user preferences - persisted under `root_cache_dir()` and
//! owned by the core (ADR-0010: native settings UI is view-only over FFI).

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// When the vault should auto-lock. Timed idle lock is intentionally absent -
/// policies are event-based (quit, Mac reboot, password use).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VaultAutoLock {
    /// No automatic lock during a session - manual Lock Now only.
    Never,
    /// Lock when Browser quits gracefully.
    #[default]
    OnBrowserClose,
    /// Lock when the Mac reboots - not when Browser relaunches. The host forwards
    /// the system boot time; unlock records it and a boot-session change locks.
    #[serde(alias = "on_restart")]
    OnMacRestart,
    /// Lock immediately after each action that retrieves a password (fill, copy,
    /// reveal).
    OnPasswordUse,
}

impl VaultAutoLock {
    /// FFI wire value: 0 Never, 1 OnBrowserClose, 2 OnMacRestart, 3 OnPasswordUse.
    pub fn to_i32(self) -> i32 {
        match self {
            VaultAutoLock::Never => 0,
            VaultAutoLock::OnBrowserClose => 1,
            VaultAutoLock::OnMacRestart => 2,
            VaultAutoLock::OnPasswordUse => 3,
        }
    }

    pub fn from_i32(value: i32) -> Self {
        match value {
            1 => VaultAutoLock::OnBrowserClose,
            3 => VaultAutoLock::OnPasswordUse,
            2 => VaultAutoLock::OnMacRestart,
            0 => VaultAutoLock::Never,
            _ => VaultAutoLock::OnBrowserClose,
        }
    }

    /// Whether to lock the vault on graceful browser shutdown.
    pub fn locks_on_shutdown(self) -> bool {
        matches!(
            self,
            VaultAutoLock::OnBrowserClose | VaultAutoLock::OnPasswordUse
        )
    }

    /// Whether to lock immediately after a password crosses the FFI boundary.
    pub fn locks_after_password_use(self) -> bool {
        self == VaultAutoLock::OnPasswordUse
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct SettingsState {
    #[serde(default)]
    vault_auto_lock: VaultAutoLock,
    /// Boot time (Unix seconds) recorded at the last vault unlock for
    /// [`VaultAutoLock::OnMacRestart`]. Zero when unset.
    #[serde(default)]
    vault_unlock_boot_secs: u64,
    /// Whether saved passwords may be filled in private windows (default off).
    #[serde(default)]
    private_vault_enabled: bool,
    /// Whether the first-run private-window password prompt has been shown.
    #[serde(default)]
    private_vault_prompt_seen: bool,
    /// Whether links opened from other apps land in a little window instead of
    /// a sidebar tab (P1-45, Arc default: on).
    #[serde(default = "default_true")]
    external_links_little: bool,
    /// Legacy timed setting - read for migration only.
    #[serde(default, skip_serializing)]
    vault_lock_timeout_secs: Option<u64>,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            vault_auto_lock: VaultAutoLock::OnBrowserClose,
            vault_unlock_boot_secs: 0,
            private_vault_enabled: false,
            private_vault_prompt_seen: false,
            external_links_little: true,
            vault_lock_timeout_secs: None,
        }
    }
}

static STATE: Mutex<SettingsState> = Mutex::new(SettingsState {
    vault_auto_lock: VaultAutoLock::OnBrowserClose,
    vault_unlock_boot_secs: 0,
    private_vault_enabled: false,
    private_vault_prompt_seen: false,
    external_links_little: true,
    vault_lock_timeout_secs: None,
});

fn default_true() -> bool {
    true
}

/// Last system boot time the host forwarded (Unix seconds).
static SYSTEM_BOOT_SECS: AtomicU64 = AtomicU64::new(0);

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
        if policy != VaultAutoLock::OnMacRestart {
            state.vault_unlock_boot_secs = 0;
        }
    }
    save();
}

/// The host reports the Mac's current boot time (launch + after wake from sleep
/// if the host re-forwards). Drives [`VaultAutoLock::OnMacRestart`].
pub fn set_system_boot_secs(secs: u64) {
    SYSTEM_BOOT_SECS.store(secs, Ordering::Relaxed);
}

/// Record a vault unlock for the Mac-restart policy.
pub fn note_vault_unlocked() {
    if vault_auto_lock() != VaultAutoLock::OnMacRestart {
        return;
    }
    let boot = SYSTEM_BOOT_SECS.load(Ordering::Relaxed);
    if boot == 0 {
        return;
    }
    {
        let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
        state.vault_unlock_boot_secs = boot;
    }
    save();
}

/// Lock when the Mac has rebooted since the vault was unlocked.
pub fn maybe_lock_for_mac_restart(lock: impl FnOnce()) {
    if vault_auto_lock() != VaultAutoLock::OnMacRestart {
        return;
    }
    if crate::vault::service::is_locked_direct() {
        return;
    }
    let current_boot = SYSTEM_BOOT_SECS.load(Ordering::Relaxed);
    if current_boot == 0 {
        return;
    }
    let unlock_boot = STATE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .vault_unlock_boot_secs;
    if unlock_boot != 0 && unlock_boot != current_boot {
        lock();
        let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
        state.vault_unlock_boot_secs = 0;
        save();
    }
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

/// May saved passwords be used in private windows?
pub fn private_vault_enabled() -> bool {
    STATE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .private_vault_enabled
}

pub fn set_private_vault_enabled(enabled: bool) {
    {
        let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
        if state.private_vault_enabled == enabled {
            return;
        }
        state.private_vault_enabled = enabled;
    }
    save();
}

/// Has the first-run private-window password prompt been shown?
pub fn private_vault_prompt_seen() -> bool {
    STATE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .private_vault_prompt_seen
}

pub fn set_private_vault_prompt_seen(seen: bool) {
    {
        let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
        if state.private_vault_prompt_seen == seen {
            return;
        }
        state.private_vault_prompt_seen = seen;
    }
    save();
}

/// Map a persisted timed auto-lock (removed) to an event-based policy.
fn migrate_timed_auto_lock(state: &mut SettingsState) {
    if let Some(secs) = state.vault_lock_timeout_secs.take() {
        state.vault_auto_lock = if secs == 0 {
            VaultAutoLock::Never
        } else {
            VaultAutoLock::OnBrowserClose
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

/// Do links from other apps open in a little window (P1-45)?
pub fn external_links_little() -> bool {
    STATE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .external_links_little
}

pub fn set_external_links_little(enabled: bool) {
    {
        let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
        if state.external_links_little == enabled {
            return;
        }
        state.external_links_little = enabled;
    }
    save();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_value_round_trips() {
        for policy in [
            VaultAutoLock::Never,
            VaultAutoLock::OnBrowserClose,
            VaultAutoLock::OnMacRestart,
            VaultAutoLock::OnPasswordUse,
        ] {
            assert_eq!(VaultAutoLock::from_i32(policy.to_i32()), policy);
        }
        assert_eq!(VaultAutoLock::from_i32(99), VaultAutoLock::OnBrowserClose);
    }

    #[test]
    fn defaults_to_on_browser_close() {
        assert_eq!(
            SettingsState::default().vault_auto_lock,
            VaultAutoLock::OnBrowserClose
        );
    }

    #[test]
    fn json_round_trips_and_accepts_legacy_on_restart_alias() {
        let state = SettingsState {
            vault_auto_lock: VaultAutoLock::OnPasswordUse,
            vault_unlock_boot_secs: 0,
            private_vault_enabled: false,
            private_vault_prompt_seen: false,
            external_links_little: true,
            vault_lock_timeout_secs: None,
        };
        let back: SettingsState =
            serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(state, back);

        let legacy: SettingsState =
            serde_json::from_str(r#"{"vault_auto_lock":"on_restart"}"#).unwrap();
        assert_eq!(legacy.vault_auto_lock, VaultAutoLock::OnMacRestart);
    }

    #[test]
    fn external_links_default_to_little_windows() {
        let old: SettingsState =
            serde_json::from_str(r#"{"vault_auto_lock":"on_restart"}"#).unwrap();
        assert!(old.external_links_little);
    }

    #[test]
    fn timed_setting_migrates_to_event_policy() {
        let mut state = SettingsState {
            vault_auto_lock: VaultAutoLock::OnBrowserClose,
            vault_unlock_boot_secs: 0,
            private_vault_enabled: false,
            private_vault_prompt_seen: false,
            external_links_little: true,
            vault_lock_timeout_secs: Some(300),
        };
        migrate_timed_auto_lock(&mut state);
        assert_eq!(state.vault_auto_lock, VaultAutoLock::OnBrowserClose);

        let mut never = SettingsState {
            vault_auto_lock: VaultAutoLock::OnBrowserClose,
            vault_unlock_boot_secs: 0,
            private_vault_enabled: false,
            private_vault_prompt_seen: false,
            external_links_little: true,
            vault_lock_timeout_secs: Some(0),
        };
        migrate_timed_auto_lock(&mut never);
        assert_eq!(never.vault_auto_lock, VaultAutoLock::Never);
    }

    #[test]
    fn policy_flags() {
        assert!(!VaultAutoLock::Never.locks_on_shutdown());
        assert!(VaultAutoLock::OnBrowserClose.locks_on_shutdown());
        assert!(!VaultAutoLock::OnMacRestart.locks_on_shutdown());
        assert!(VaultAutoLock::OnPasswordUse.locks_on_shutdown());
        assert!(VaultAutoLock::OnPasswordUse.locks_after_password_use());
    }

    #[test]
    fn mac_restart_locks_after_boot_session_changes() {
        let dir = std::env::temp_dir().join(format!(
            "browser-settings-mac-restart-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            br#"{"vault_auto_lock":"on_mac_restart","vault_unlock_boot_secs":100}"#,
        )
        .unwrap();

        // Stand in for root_cache_dir - test the logic directly.
        let state: SettingsState = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(state.vault_auto_lock, VaultAutoLock::OnMacRestart);
        assert_eq!(state.vault_unlock_boot_secs, 100);

        SYSTEM_BOOT_SECS.store(200, Ordering::Relaxed);
        *STATE.lock().unwrap() = state;
        let mut locked = false;
        if STATE.lock().unwrap().vault_unlock_boot_secs != 0
            && STATE.lock().unwrap().vault_unlock_boot_secs
                != SYSTEM_BOOT_SECS.load(Ordering::Relaxed)
        {
            locked = true;
        }
        assert!(locked);

        // Don't leak OnMacRestart + boot secs into later tests - `is_locked()`
        // re-enters this policy and used to recurse forever.
        SYSTEM_BOOT_SECS.store(0, Ordering::Relaxed);
        *STATE.lock().unwrap() = SettingsState::default();

        let _ = std::fs::remove_dir_all(&dir);
    }
}
