//! Vault service — the process-global [`VaultManager`] + a flat, FFI-facing API.
//!
//! Mirrors how [`embed`](crate::embed) / [`tabs`](crate::tabs) expose the browser:
//! the native app never touches the vault types directly; it calls these free
//! functions (through the thin `browser_vault_*` FFI), which own the one global
//! manager. Returns are FFI-friendly — bools, counts, `String`/JSON, and
//! `Result<_, String>` (the error already rendered for display).
//!
//! **Threading:** the networked calls ([`unlock_online`], [`import_csv`],
//! [`import_bitwarden_json`]) and [`unlock_offline`] (heavy KDF) **block** and
//! must be called off the UI thread. They hold the global lock while running, so
//! a concurrent [`is_locked`] blocks until they finish — fine for the modal
//! unlock/import flows; a non-blocking (callback) variant is a later refinement
//! if a caller needs to poll during an unlock.
//!
//! **Secrets:** [`lookup`] / [`list`] return metadata only — never passwords.
//! A password crosses the boundary solely via [`password_for`] on an explicit
//! user gesture (ADR-0017).

use crate::vault::auth::{
    AuthError, Device, Grant, ServerConfig, TwoFactor, TwoFactorProvider, VaultClient,
};
use crate::vault::http::UreqTransport;
use crate::vault::import::{parse_bitwarden_json, parse_csv, ImportRecord};
use crate::vault::manager::{VaultError, VaultManager};
use crate::vault::sync::Login;
use serde::Serialize;
use std::sync::Mutex;

/// This device's name as it appears in the Bitwarden account's device list.
const DEVICE_NAME: &str = "Browser (macOS)";

static VAULT: Mutex<Option<VaultManager<UreqTransport>>> = Mutex::new(None);

/// Run `f` against the global manager, or `None` if it was never [`init`]ed.
fn with<R>(f: impl FnOnce(&mut VaultManager<UreqTransport>) -> R) -> Option<R> {
    let mut guard = VAULT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.as_mut().map(f)
}

/// A login as the UI lists it — **no password** (see [`password_for`]).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LoginView {
    pub id: String,
    pub name: String,
    pub username: Option<String>,
    pub uris: Vec<String>,
}

impl LoginView {
    fn of(login: &Login) -> Self {
        LoginView {
            id: login.id.clone(),
            name: login.name.clone(),
            username: login.username.clone(),
            uris: login.uris.iter().map(|u| u.uri.clone()).collect(),
        }
    }
}

/// Initialize (or replace) the global manager for `server_url`, persisting the
/// encrypted cache + a stable device id under `cache_dir`.
pub fn init(server_url: &str, cache_dir: &str) {
    let manager = VaultManager::new(
        VaultClient::new(UreqTransport::new(), ServerConfig::for_input(server_url)),
        load_or_create_device(cache_dir),
        Some(format!("{cache_dir}/vault.json")),
    );
    *VAULT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(manager);
}

// ─── Setup / configuration ─────────────────────────────────────────────────
//
// The vault is set up once (server URL + account); the choice is persisted to
// `root_cache_dir()/vault-config.json` so later launches auto-init ([`start`]).

const CONFIG_FILE: &str = "vault-config.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct VaultConfig {
    /// The self-hosted base URL, or empty for the default Bitwarden cloud.
    #[serde(default)]
    server: String,
    #[serde(default)]
    email: String,
}

fn cache_dir() -> String {
    crate::root_cache_dir()
}

fn config_path() -> String {
    format!("{}/{}", cache_dir(), CONFIG_FILE)
}

fn load_config() -> Option<VaultConfig> {
    let bytes = std::fs::read(config_path()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_config(config: &VaultConfig) {
    let Ok(json) = serde_json::to_vec(config) else {
        return;
    };
    let path = config_path();
    let tmp = format!("{path}.tmp");
    if let Err(err) = std::fs::write(&tmp, &json).and_then(|()| std::fs::rename(&tmp, &path)) {
        eprintln!("[vault] failed to write {path}: {err}");
    }
}

/// Whether the vault has been set up on this machine (a server is configured).
pub fn is_configured() -> bool {
    load_config().is_some()
}

/// Auto-initialize the manager from persisted config at startup — leaving it
/// locked and ready to unlock (from cache offline, or by re-login online). No-op
/// when the vault was never set up. Called by [`crate::embed::initialize`].
pub fn start() {
    if let Some(config) = load_config() {
        init(&config.server, &cache_dir());
    }
}

/// The result of an online unlock/setup attempt, shaped for the UI.
pub enum UnlockOutcome {
    /// The vault is open.
    Unlocked,
    /// A second factor is required before the vault will open. `provider_tag` is
    /// the challenge kind (`"email"` / `"authenticator"` / `"yubikey"` / …); for
    /// `"email"` the code has already been dispatched to the user's inbox.
    TwoFactorRequired { provider_tag: &'static str },
    /// The attempt failed; the string is display-ready.
    Failed(String),
}

/// Set up the vault: point it at `server`, log in online, and (on success)
/// persist the config so later launches auto-init. `code` empty ⇒ first attempt;
/// a `TwoFactorRequired` outcome means the user must re-call with the code (and
/// the `provider` tag it carried). Blocks (network + KDF); call off the UI thread.
pub fn setup(
    server: &str,
    email: &str,
    password: &str,
    code: &str,
    provider: &str,
) -> UnlockOutcome {
    init(server, &cache_dir());
    let outcome = unlock_online(email, password, code, provider);
    if matches!(outcome, UnlockOutcome::Unlocked) {
        save_config(&VaultConfig {
            server: server.to_string(),
            email: email.to_string(),
        });
    }
    outcome
}

/// True when locked *or* never initialized (fail safe).
pub fn is_locked() -> bool {
    with(|m| m.is_locked()).unwrap_or(true)
}

/// Online unlock. `code` empty ⇒ no second factor supplied yet. On a two-factor
/// challenge this returns [`UnlockOutcome::TwoFactorRequired`] — and, for the
/// Email provider, dispatches the code before returning so the user has one to
/// enter. Blocks (network + KDF); call off the UI thread.
pub fn unlock_online(email: &str, password: &str, code: &str, provider: &str) -> UnlockOutcome {
    let two_factor = build_two_factor(code, provider);
    let result = with(|m| {
        m.unlock_online(
            email.to_string(),
            password.to_string(),
            Grant::Password,
            two_factor.as_ref(),
        )
    });
    match result {
        None => UnlockOutcome::Failed(not_initialized()),
        Some(Ok(())) => UnlockOutcome::Unlocked,
        Some(Err(VaultError::Auth(AuthError::TwoFactorRequired(providers)))) => {
            let primary = select_provider(&providers);
            // Email is the one provider the client must trigger — dispatch it now
            // so the user has a code to enter. Best-effort: a send failure still
            // surfaces the challenge (the UI's Resend can retry).
            if primary == TwoFactorProvider::Email {
                let _ = with(|m| m.send_email_code(email, password));
            }
            UnlockOutcome::TwoFactorRequired {
                provider_tag: primary.tag(),
            }
        }
        Some(Err(e)) => UnlockOutcome::Failed(render(e)),
    }
}

/// Re-dispatch the login two-factor email (the UI's "Resend" affordance). Blocks
/// (network + KDF); call off the UI thread.
pub fn send_email_code(email: &str, password: &str) -> Result<(), String> {
    with(|m| m.send_email_code(email, password))
        .ok_or_else(not_initialized)?
        .map_err(render)
}

/// Build the two-factor payload from a UI code + provider tag. Empty code ⇒ no
/// second factor. An unknown/empty tag defaults to Authenticator, preserving the
/// "type a TOTP up front" path.
fn build_two_factor(code: &str, provider: &str) -> Option<TwoFactor> {
    if code.is_empty() {
        return None;
    }
    Some(TwoFactor {
        provider: TwoFactorProvider::from_tag(provider).unwrap_or(TwoFactorProvider::Authenticator),
        token: code.to_string(),
        remember: false,
    })
}

/// Pick which offered provider to drive. Prefer the device-held factors
/// (authenticator, then yubikey) over email, which needs a server round-trip.
fn select_provider(providers: &[TwoFactorProvider]) -> TwoFactorProvider {
    [
        TwoFactorProvider::Authenticator,
        TwoFactorProvider::YubiKey,
        TwoFactorProvider::Email,
    ]
    .into_iter()
    .find(|p| providers.contains(p))
    .or_else(|| providers.first().copied())
    .unwrap_or(TwoFactorProvider::Email)
}

/// Offline unlock from the cache. Blocks (KDF); call off the UI thread.
pub fn unlock_offline(password: &str) -> Result<(), String> {
    with(|m| m.unlock_offline(password))
        .ok_or_else(not_initialized)?
        .map_err(render)
}

pub fn lock() {
    with(|m| m.lock());
}

/// Logins matching `host` (metadata only), best-first. Empty when locked.
pub fn lookup(host: &str) -> Vec<LoginView> {
    with(|m| {
        m.lookup(host)
            .map(|logins| logins.iter().map(|l| LoginView::of(l)).collect())
            .unwrap_or_default()
    })
    .unwrap_or_default()
}

/// All logins (metadata only). Empty when locked.
pub fn list() -> Vec<LoginView> {
    with(|m| {
        m.logins()
            .map(|logins| logins.iter().map(LoginView::of).collect())
            .unwrap_or_default()
    })
    .unwrap_or_default()
}

/// The password for login `id` — the one path a secret crosses the boundary.
/// `None` if locked or unknown.
pub fn password_for(id: &str) -> Option<String> {
    with(|m| {
        m.logins()
            .ok()
            .and_then(|logins| logins.iter().find(|l| l.id == id))
            .and_then(|l| l.password.clone())
    })
    .flatten()
}

/// A login's full detail — **includes the password** (an explicit gesture: the
/// user opened edit) and notes, so the edit form round-trips every field.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LoginDetail {
    pub id: String,
    pub name: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub uri: Option<String>,
    pub totp: Option<String>,
    pub notes: Option<String>,
}

/// Full detail for login `id` (`None` when locked/unknown) — for the edit form.
pub fn login_detail(id: &str) -> Option<LoginDetail> {
    with(|m| {
        m.logins().ok().and_then(|logins| {
            logins.iter().find(|l| l.id == id).map(|l| LoginDetail {
                id: l.id.clone(),
                name: l.name.clone(),
                username: l.username.clone(),
                password: l.password.clone(),
                uri: l.uris.first().map(|u| u.uri.clone()),
                totp: l.totp.clone(),
                notes: l.notes.clone(),
            })
        })
    })
    .flatten()
}

/// Import a CSV export → count uploaded. Blocks; call off the UI thread.
pub fn import_csv(data: &str) -> Result<usize, String> {
    let records = parse_csv(data).map_err(|e| e.to_string())?;
    with(|m| m.import(&records))
        .ok_or_else(not_initialized)?
        .map_err(render)
}

/// Import a Bitwarden JSON export → count uploaded. Blocks; call off the UI thread.
pub fn import_bitwarden_json(data: &str) -> Result<usize, String> {
    let records = parse_bitwarden_json(data).map_err(|e| e.to_string())?;
    with(|m| m.import(&records))
        .ok_or_else(not_initialized)?
        .map_err(render)
}

/// Create a new login from UI fields. Blocks; call off the UI thread.
pub fn add_login(
    name: &str,
    username: &str,
    password: &str,
    uri: &str,
    totp: &str,
    notes: &str,
) -> Result<(), String> {
    let record = record_from(name, username, password, uri, totp, notes);
    with(|m| m.add_login(&record))
        .ok_or_else(not_initialized)?
        .map_err(render)
}

/// Update an existing login `id` from UI fields. Blocks; call off the UI thread.
pub fn update_login(
    id: &str,
    name: &str,
    username: &str,
    password: &str,
    uri: &str,
    totp: &str,
    notes: &str,
) -> Result<(), String> {
    let record = record_from(name, username, password, uri, totp, notes);
    with(|m| m.update_login(id, &record))
        .ok_or_else(not_initialized)?
        .map_err(render)
}

/// Delete login `id`. Blocks; call off the UI thread.
pub fn delete_login(id: &str) -> Result<(), String> {
    with(|m| m.delete_login(id))
        .ok_or_else(not_initialized)?
        .map_err(render)
}

/// Build an `ImportRecord` (the shared login draft) from UI fields; empty
/// optionals become `None`.
fn record_from(
    name: &str,
    username: &str,
    password: &str,
    uri: &str,
    totp: &str,
    notes: &str,
) -> ImportRecord {
    let nonempty = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
    ImportRecord {
        name: name.trim().to_string(),
        username: nonempty(username),
        password: nonempty(password),
        uri: nonempty(uri),
        totp: nonempty(totp),
        notes: nonempty(notes),
    }
}

fn render(e: VaultError) -> String {
    e.to_string()
}

fn not_initialized() -> String {
    "vault not initialized".to_string()
}

/// Debug-only: seed an unlocked vault with one demo login for `localhost`, for
/// the autofill end-to-end harness (triggered by the `BROWSER_VAULT_DEMO` env
/// var in [`crate::embed::initialize`]). Bypasses network/crypto — the real
/// unlock path is live-validated separately.
#[cfg(debug_assertions)]
pub fn seed_demo_vault() {
    use crate::vault::sync::{LoginUri, UriMatch};
    // Two logins for `localhost` so the autofill picker's multi-match path is
    // exercised end-to-end (P2-16), not just single-match fill.
    let localhost = || LoginUri {
        uri: "http://localhost".to_string(),
        match_type: UriMatch::Domain,
    };
    let logins = vec![
        Login {
            id: "demo".to_string(),
            name: "Demo Login".to_string(),
            username: Some("octocat@example.com".to_string()),
            password: Some("correct horse battery staple".to_string()),
            uris: vec![localhost()],
            totp: None,
            notes: None,
            folder: None,
        },
        Login {
            id: "demo-work".to_string(),
            name: "Work Account".to_string(),
            username: Some("work@example.com".to_string()),
            password: Some("hunter2-work-pw".to_string()),
            uris: vec![localhost()],
            totp: None,
            notes: None,
            folder: None,
        },
    ];
    let mut guard = VAULT.lock().unwrap_or_else(|p| p.into_inner());
    if guard.is_none() {
        *guard = Some(VaultManager::new(
            VaultClient::new(
                UreqTransport::new(),
                ServerConfig::self_hosted("http://localhost"),
            ),
            Device::generate("demo"),
            None,
        ));
    }
    if let Some(manager) = guard.as_mut() {
        manager.unlock_with_logins(logins);
    }
    eprintln!("[autofill] demo vault seeded (localhost → octocat@example.com, work@example.com)");
}

/// Load the persisted device id, or generate + persist a fresh one. The id is
/// not a secret (it just names this device to the server), so a plain file is fine.
fn load_or_create_device(cache_dir: &str) -> Device {
    let path = format!("{cache_dir}/vault-device");
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim();
        if !id.is_empty() {
            return Device::with_identifier(id.to_string(), DEVICE_NAME);
        }
    }
    let device = Device::generate(DEVICE_NAME);
    if let Err(err) = std::fs::write(&path, &device.identifier) {
        eprintln!("[vault] failed to persist device id to {path}: {err}");
    }
    device
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::sync::{LoginUri, UriMatch};

    #[test]
    fn login_view_drops_the_password_and_flattens_uris() {
        let login = Login {
            id: "c1".to_string(),
            name: "Example".to_string(),
            username: Some("alice".to_string()),
            password: Some("s3cret".to_string()),
            uris: vec![
                LoginUri {
                    uri: "https://example.com".to_string(),
                    match_type: UriMatch::Domain,
                },
                LoginUri {
                    uri: "https://login.example.com".to_string(),
                    match_type: UriMatch::Host,
                },
            ],
            totp: None,
            notes: None,
            folder: None,
        };
        let view = LoginView::of(&login);
        assert_eq!(view.id, "c1");
        assert_eq!(view.username.as_deref(), Some("alice"));
        assert_eq!(
            view.uris,
            vec!["https://example.com", "https://login.example.com"]
        );
        // The serialized view must not carry the password.
        let json = serde_json::to_string(&view).unwrap();
        assert!(
            !json.contains("s3cret"),
            "password leaked into LoginView JSON: {json}"
        );
        assert!(!json.contains("password"));
    }

    #[test]
    fn device_id_persists_and_reloads() {
        let dir = std::env::temp_dir().join("browser-vault-device-test");
        std::fs::create_dir_all(&dir).unwrap();
        let cache_dir = dir.to_str().unwrap();
        std::fs::remove_file(format!("{cache_dir}/vault-device")).ok();

        let first = load_or_create_device(cache_dir);
        let second = load_or_create_device(cache_dir);
        assert_eq!(
            first.identifier, second.identifier,
            "device id is stable across calls"
        );
        assert_eq!(first.identifier.len(), 36, "UUIDv4");
        assert_eq!(second.name, DEVICE_NAME);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn operations_are_safe_before_init() {
        // Without init() the global is None — nothing panics, and it reads locked.
        // (This runs in the shared test process; init() is never called here.)
        assert!(is_locked());
        assert!(lookup("example.com").is_empty());
        assert!(list().is_empty());
        assert_eq!(password_for("nope"), None);
        assert_eq!(
            unlock_offline("pw"),
            Err("vault not initialized".to_string())
        );
    }

    #[test]
    fn select_provider_prefers_device_factors_over_email() {
        use TwoFactorProvider::*;
        // Email alone → email (the user's case: a code must be dispatched).
        assert_eq!(select_provider(&[Email]), Email);
        // Authenticator wins over email when both are offered (no round-trip).
        assert_eq!(select_provider(&[Email, Authenticator]), Authenticator);
        // YubiKey outranks email but not authenticator.
        assert_eq!(select_provider(&[Email, YubiKey]), YubiKey);
        // Unknown-only set falls back to the first offered.
        assert_eq!(select_provider(&[Other(9)]), Other(9));
    }

    #[test]
    fn build_two_factor_maps_code_and_provider() {
        // Empty code ⇒ no second factor.
        assert!(build_two_factor("", "email").is_none());
        // A code under an explicit provider tag.
        let tf = build_two_factor("123456", "email").unwrap();
        assert_eq!(tf.provider, TwoFactorProvider::Email);
        assert_eq!(tf.token, "123456");
        // No/unknown tag defaults to Authenticator (the TOTP-up-front path).
        assert_eq!(
            build_two_factor("999", "").unwrap().provider,
            TwoFactorProvider::Authenticator
        );
    }
}
