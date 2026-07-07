//! Embedded mode — host CEF inside a native app that owns the window & run loop.
//!
//! Where [`crate::run`] creates its own CEF Views window, embedded mode lets a
//! native UI (the macOS SwiftUI app) own the `NSWindow` and hand CEF a child
//! `NSView` to render into (the "Option B" hosting model from the architecture
//! spike). The browser is a *windowed* Alloy (chromeless) browser: CEF inserts
//! its own GPU-accelerated `NSView` as a subview of the host-provided parent
//! view, so Chromium's compositor presents directly — no off-screen rendering,
//! no frame copies. The host drives the lifecycle across the FFI boundary:
//!
//!   1. [`initialize`] — load CEF, run the subprocess check, init CEF.
//!   2. [`create_browser`] — create an Alloy browser parented to a
//!      host-provided `NSView`.
//!   3. [`run_message_loop`] — hand the run loop to CEF.
//!   4. [`shutdown`] — tear down CEF.
//!
//! Helper (renderer/GPU/utility) subprocesses are unchanged: they are the
//! separate `browser_helper` executable, which routes through [`crate::run`]'s
//! helper branch.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use cef::args::Args;
use cef::*;
// Aliased to disambiguate from the local `run_message_loop`/`shutdown` wrappers.
use cef::{
    initialize as cef_initialize, run_message_loop as cef_run_message_loop,
    shutdown as cef_shutdown,
};

use crate::client::BrowserClient;
use crate::tabs::{self, EventCallback, TabId};
use crate::{mac, root_cache_dir};

/// Keeps the CEF framework loaded for the lifetime of the process.
static LOADER: OnceLock<mac::LibraryLoader> = OnceLock::new();

thread_local! {
    /// Retain each browser's `Client` for the process lifetime, matching the
    /// standalone path (which holds its client in the process handler). CEF's
    /// C API convention is that the callee add-refs what it keeps, but the
    /// proven-working paths in this codebase all retain the client on our
    /// side too, so embedded mode does the same. Main-thread only (as is
    /// `create_browser`), hence `thread_local`.
    static CLIENTS: RefCell<Vec<Client>> = const { RefCell::new(Vec::new()) };
}

/// Host callback for find-in-page results, invoked on the UI (main) thread with
/// the total match `count`, the 1-based `active_match_ordinal`, and whether this
/// is the `final_update` for the current search. The macOS app registers one to
/// drive the find bar's "n of m" counter. `None` (the default, and the case in
/// standalone mode) makes [`dispatch_find_result`] a no-op.
pub type FindResultCallback =
    extern "C" fn(count: i32, active_match_ordinal: i32, final_update: bool);

static FIND_RESULT_CALLBACK: Mutex<Option<FindResultCallback>> = Mutex::new(None);

/// Register (or clear, with `None`) the host's find-in-page result callback.
pub fn set_find_result_callback(callback: Option<FindResultCallback>) {
    *FIND_RESULT_CALLBACK
        .lock()
        .expect("find callback lock poisoned") = callback;
}

/// Forward a find result to the host callback. Called from the find handler on
/// the UI thread (which is the app's main thread in embedded mode, so the host
/// receives it there). Copies the pointer out before invoking so the lock isn't
/// held across the call into host code.
pub(crate) fn dispatch_find_result(count: i32, active_match_ordinal: i32, final_update: bool) {
    let callback = *FIND_RESULT_CALLBACK
        .lock()
        .expect("find callback lock poisoned");
    if let Some(callback) = callback {
        callback(count, active_match_ordinal, final_update);
    }
}

// App for embedded mode. Unlike the standalone `BrowserApp` it has no browser
// process handler: the host creates browsers itself via [`create_browser`]
// after [`initialize`] returns (CEF invokes `on_context_initialized` inside
// `cef_initialize` when CEF runs the message loop on this thread, so the
// context is ready by then).
wrap_app! {
    pub struct EmbedApp;

    impl App {
        fn on_before_command_line_processing(
            &self,
            process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            let Some(command_line) = command_line else {
                return;
            };

            // Debug builds use Chromium's in-memory mock keychain — see
            // `BrowserApp::on_before_command_line_processing` for the rationale.
            #[cfg(debug_assertions)]
            command_line.append_switch(Some(&CefString::from("use-mock-keychain")));

            // CDP automation surface (P0-4): browser process only (helpers ignore
            // `--remote-debugging-port`), only when the user opted in, loopback-bound.
            let is_browser_process = process_type
                .map(|t| t.to_string().is_empty())
                .unwrap_or(true);
            if is_browser_process {
                for (name, value) in crate::cdp::switches(&crate::cdp::config()) {
                    command_line.append_switch_with_value(
                        Some(&CefString::from(name)),
                        Some(&CefString::from(value.as_str())),
                    );
                }
            }
        }
    }
}

/// Load CEF and initialize it for embedded hosting. Returns `false` on failure.
///
/// Must be called once, on the main thread, before the host creates any window.
pub fn initialize() -> bool {
    let loader = mac::load_cef_framework();
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    mac::setup_simple_application();
    let _ = LOADER.set(loader);

    let args = Args::new();
    let main_args = args.as_main_args();

    // The host app process is always the browser process (helpers are the
    // separate `browser_helper` executable), so `execute_process` returns -1.
    if execute_process(Some(main_args), None, std::ptr::null_mut()) >= 0 {
        return false;
    }

    // Read the persisted CDP consent before init — the switch is applied during
    // `cef_initialize` (in `on_before_command_line_processing`).
    crate::cdp::load_persisted();

    // Bring up the adblock engine (seed synchronously, full lists in the
    // background) before any browser is created, so the request-path filter is
    // live from the first navigation (ADR-0020, spec §11). In debug, a
    // `BROWSER_ADBLOCK_DEMO` env installs a stable engine with a localhost test
    // cosmetic rule (no background refresh) so the P2-21 smoke is deterministic.
    #[cfg(debug_assertions)]
    if std::env::var_os("BROWSER_ADBLOCK_DEMO").is_some() {
        crate::adblock::init_demo("localhost##.ad-banner\nlocalhost###ad-box");
    } else {
        crate::adblock::init();
    }
    #[cfg(not(debug_assertions))]
    crate::adblock::init();

    let mut app = EmbedApp::new();
    let settings = Settings {
        no_sandbox: !cfg!(feature = "sandbox") as _,
        root_cache_path: CefString::from(root_cache_dir().as_str()),
        browser_subprocess_path: CefString::from(helper_subprocess_path().as_str()),
        ..Default::default()
    };
    let ok = cef_initialize(
        Some(main_args),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    ) == 1;

    // Open the history/favicon store next to the CEF cache. A failure here is
    // non-fatal — the browser just runs without autocomplete suggestions.
    crate::history::init(&format!("{}/history.sqlite", root_cache_dir()));

    // Auto-initialize the vault from its persisted config (if set up before), so
    // it is ready to unlock; a no-op on first run (the UI shows setup).
    crate::vault::service::start();

    // Register vault autofill as a rider on the injection substrate (P2-15,
    // ADR-0017). Unconditional: the detect script is cheap and matches nothing
    // while the vault is locked. This is the substrate's first production consumer.
    crate::vault::autofill::register();

    // Register cosmetic ad-block as a substrate rider too (P2-21, ADR-0020):
    // element-hiding CSS injected at document-start, no flash.
    crate::adblock::cosmetic::register();

    // Register cookie-consent (P2-22/P2-25, ADR-0026): detect known CMPs and click
    // "reject all", plus hide the long-tail banners — gated on its own toggle,
    // whose persisted state loads here before the first browser.
    crate::consent::load_persisted();
    crate::consent::init_lists();
    crate::consent::register();

    // Load the appearance preference (Light/Dark/System, ADR-0030) before the
    // first browser. It is *applied* once the host forwards the OS appearance
    // (`browser_theme_set_system_dark`) early in window setup, so the global
    // request context is themed before any tab hydrates.
    crate::theme::load_persisted();
    crate::settings::load_persisted();

    // Autofill end-to-end harness: seed an unlocked demo vault (debug only,
    // overriding any configured vault for the demo).
    #[cfg(debug_assertions)]
    if std::env::var_os("BROWSER_VAULT_DEMO").is_some() {
        crate::vault::service::seed_demo_vault();
    }

    // Injection-substrate end-to-end harness: register a demo consumer that
    // exercises the isolated-world round-trip (debug only, P2-14/ADR-0017).
    #[cfg(debug_assertions)]
    if std::env::var_os("BROWSER_INJECT_DEMO").is_some() {
        crate::inject::register_demo_consumer();
    }

    // Create the tab/space model, restoring the persisted session (ADR-0013).
    // Restored tabs are suspended — the host hydrates each one (creating its
    // CEF browser via `restore_tab`) the first time it is shown. With no saved
    // session the app launches to the empty state.
    tabs::init(&format!("{}/session.json", root_cache_dir()));

    mac::setup_app_delegate();
    ok
}

/// Absolute path to the helper-subprocess executable inside the host app bundle.
///
/// With an empty `browser_subprocess_path`, CEF derives the helper location on
/// macOS from the main bundle name — `Contents/Frameworks/<App> Helper.app`.
/// The embedded host bundle is `Browser.app`, but the bundled helper (reused
/// from the standalone layout) is `browser Helper.app`; that case difference
/// resolves only on case-insensitive volumes, so we pin the path explicitly:
///
///   `Browser.app/Contents/MacOS/Browser`
///     → `Browser.app/Contents/Frameworks/browser Helper.app/Contents/MacOS/browser Helper`
///
/// The single generic helper serves every subprocess role (the sandbox is off in
/// dev, so no role-specific entitlement separation is needed yet).
fn helper_subprocess_path() -> String {
    let exe = std::env::current_exe().expect("failed to resolve current executable path");
    let contents = exe
        .parent() // Contents/MacOS
        .and_then(std::path::Path::parent) // Contents
        .expect("host executable is not inside a .app bundle");
    contents
        .join("Frameworks/browser Helper.app/Contents/MacOS/browser Helper")
        .to_string_lossy()
        .into_owned()
}

/// Create a windowed Alloy (chromeless) browser as a child of `parent_view`.
///
/// CEF inserts its own `NSView` as a subview of `parent_view` and renders into
/// it directly. The host keeps that subview sized to the container (see the
/// `layout()` override in the Swift `CEFContainerView`). Must be called on the
/// main thread after [`initialize`]; creation completes asynchronously once
/// the message loop is pumping.
///
/// # Safety
/// `parent_view` must be a valid `NSView` pointer that outlives the browser.
pub unsafe fn create_browser(parent_view: *mut c_void, width: i32, height: i32, url: &str) -> bool {
    let bounds = Rect {
        x: 0,
        y: 0,
        width: width.max(1),
        height: height.max(1),
    };
    let window_info = WindowInfo {
        runtime_style: RuntimeStyle::ALLOY,
        ..Default::default()
    }
    .set_as_child(parent_view, &bounds);

    let client = BrowserClient::create(RuntimeStyle::ALLOY);
    CLIENTS.with(|clients| clients.borrow_mut().push(client.clone()));
    let mut client = Some(client);

    let settings = BrowserSettings::default();
    let url = CefString::from(url);
    browser_host_create_browser(
        Some(&window_info),
        client.as_mut(),
        Some(&url),
        Some(&settings),
        None,
        None,
    ) == 1
}

// ─── Tab commands ────────────────────────────────────────────────────────
//
// The native app drives the tab lifecycle over the FFI: it owns one `NSView`
// per tab and calls these. Events flow back the other way, over the callback
// registered with [`set_event_callback`] (ADR-0003).

/// Register (or clear, with `None`) the host's tab/space event callback.
pub fn set_event_callback(callback: Option<EventCallback>) {
    tabs::set_event_callback(callback);
}

/// Create a windowed Alloy tab as a child of `parent_view`, register it in the
/// tab model (active space), and make it active. Returns the new tab's id (CEF's
/// browser identifier), or a negative value on failure.
///
/// Unlike [`create_browser`], this uses CEF's **synchronous** creation path, so
/// the tab id is known before returning — no async create→event correlation to
/// get wrong, and popups / DevTools (which are never registered here) can't be
/// mistaken for tabs. Safe with launch-to-empty: the first tab is created while
/// the message loop is already pumping and the window is visible. Main thread
/// only, after [`initialize`].
///
/// # Safety
/// `parent_view` must be a valid `NSView` pointer that outlives the tab.
pub unsafe fn create_tab(parent_view: *mut c_void, width: i32, height: i32, url: &str) -> TabId {
    let Some(browser) = create_tab_browser(parent_view, width, height, url) else {
        return -1;
    };
    tabs::register_tab(browser, url)
}

/// Hydrate a suspended tab restored from the persisted session (ADR-0013):
/// create its CEF browser as a child of `parent_view`, loading the tab's saved
/// URL. The counterpart of [`create_tab`] for tabs that already exist in the
/// model and only lack their engine side. Returns `false` when `id` is not a
/// suspended tab or creation fails. Main thread only, after [`initialize`].
///
/// # Safety
/// `parent_view` must be a valid `NSView` pointer that outlives the tab.
pub unsafe fn restore_tab(id: TabId, parent_view: *mut c_void, width: i32, height: i32) -> bool {
    let Some(url) = tabs::suspended_url(id) else {
        return false;
    };
    let Some(browser) = create_tab_browser(parent_view, width, height, &url) else {
        return false;
    };
    tabs::attach_browser(id, browser);
    true
}

/// Shared synchronous browser creation for [`create_tab`] / [`restore_tab`]:
/// a windowed Alloy browser parented to `parent_view`, its client retained for
/// the process lifetime.
///
/// # Safety
/// `parent_view` must be a valid `NSView` pointer that outlives the browser.
unsafe fn create_tab_browser(
    parent_view: *mut c_void,
    width: i32,
    height: i32,
    url: &str,
) -> Option<Browser> {
    let bounds = Rect {
        x: 0,
        y: 0,
        width: width.max(1),
        height: height.max(1),
    };
    let window_info = WindowInfo {
        runtime_style: RuntimeStyle::ALLOY,
        ..Default::default()
    }
    .set_as_child(parent_view, &bounds);

    let client = BrowserClient::create(RuntimeStyle::ALLOY);
    CLIENTS.with(|clients| clients.borrow_mut().push(client.clone()));
    let mut client = Some(client);

    let settings = BrowserSettings::default();
    let cef_url = CefString::from(url);
    browser_host_create_browser_sync(
        Some(&window_info),
        client.as_mut(),
        Some(&cef_url),
        Some(&settings),
        None,
        None,
    )
}

/// Make tab `id` the active tab (of its space). The host reacts to the resulting
/// `StructureChanged` by showing that tab's view. Main thread only.
pub fn select_tab(id: TabId) {
    tabs::select_tab(id);
}

/// Give keyboard focus to the active tab's web view, if any.
///
/// Used by the host to deterministically return focus to the page after a
/// transient chrome overlay (the command bar) closes. Without this, focus-return
/// depends on AppKit restoring the right first responder — which fails when the
/// pre-overlay responder was something else (e.g. a click into the sidebar left
/// the bare `NSWindow` as responder), silently dropping keystrokes. Mirrors the
/// tab-switch focus path (`tabs::set_visible`). No-op in the empty state (no
/// active browser). Main thread only.
pub fn focus_active_tab() {
    let Some(browser) = main_browser() else {
        return;
    };
    let Some(host) = browser.host() else {
        return;
    };
    eprintln!(
        "[focus] host set_focus (overlay close) tab={}",
        browser.identifier()
    );
    host.set_focus(1);
}

/// Fill the active tab's login form from vault login `id` — the button-driven
/// autofill path (P2-34, ADR-0029). The credential crosses into the page on this
/// **explicit gesture** (the user picked the entry in the URL-bar popover), so we
/// fetch it here and evaluate a self-contained, lenient fill into the main frame's
/// main world. No-op when the vault is locked / `id` is unknown, or there is no
/// active tab. Main thread only.
pub fn vault_fill_active_tab(id: &str) {
    let Some(detail) = crate::vault::service::login_detail(id) else {
        return;
    };
    let Some(password) = detail.password else {
        return;
    };
    let Some(browser) = main_browser() else {
        return;
    };
    let Some(host) = browser.host() else {
        return;
    };
    let expr = crate::vault::autofill::fill_expression(detail.username.as_deref(), &password);
    crate::inject::evaluate_main_world(&host, &expr);
}

/// Close tab `id`: the tab model records it for reopen, removes it (repairing
/// active), and routes its browser through CEF's close flow. Main thread only.
pub fn close_tab(id: TabId) {
    tabs::close_tab(id);
}

/// Reopen the most recently closed tab (⌘⇧T): the tab model restores it into its
/// original space / section / slot as the active tab, and the host hydrates it
/// on the resulting `StructureChanged` (lazy restore, ADR-0013). Returns the
/// reopened tab id, or a negative value when nothing is left to reopen. Main
/// thread only.
pub fn reopen_closed_tab() -> TabId {
    tabs::reopen_closed_tab()
}

/// Whether there is a closed tab to reopen — backs the menu item's enabled
/// state. Main thread only.
pub fn can_reopen_closed_tab() -> bool {
    tabs::can_reopen_closed()
}

/// Pin (persist) or unpin tab `id`. Unpinning returns it to "today".
pub fn set_tab_pinned(id: TabId, pinned: bool) {
    tabs::set_pinned(id, pinned);
}

/// Move tab `id` into folder `folder` (negative ⇒ out of any folder). Moving
/// into a folder auto-pins the tab.
pub fn move_tab_to_folder(id: TabId, folder: i32) {
    tabs::move_to_folder(id, (folder >= 0).then_some(folder));
}

/// Drag-and-drop relocation: move tab `id` into a destination section —
/// pinned/loose (`pinned`) or a folder (`folder`, negative ⇒ no folder) — and
/// position it just before tab `before` (negative ⇒ the section's end). A folder
/// destination auto-pins. One structural change; main thread only.
pub fn move_tab(id: TabId, pinned: bool, folder: i32, before: i32) {
    tabs::move_tab(
        id,
        pinned,
        (folder >= 0).then_some(folder),
        (before >= 0).then_some(before),
    );
}

/// Create an empty folder (named `name`) in the active space; returns its id.
pub fn create_folder(name: &str) -> i32 {
    tabs::create_folder(name)
}

/// Delete folder `id`, keeping its tabs (dropped to loose pinned).
pub fn delete_folder(id: i32) {
    tabs::delete_folder(id);
}

/// Rename folder `id`.
pub fn rename_folder(id: i32, name: &str) {
    tabs::rename_folder(id, name);
}

/// Create a new empty space (named `name`, with emoji `icon`) and switch to it;
/// returns its id.
pub fn create_space(name: &str, icon: &str) -> i32 {
    tabs::create_space(name, icon)
}

/// Switch to space `id`, restoring its remembered active tab.
pub fn select_space(id: i32) {
    tabs::select_space(id);
}

/// Rename space `id`.
pub fn rename_space(id: i32, name: &str) {
    tabs::rename_space(id, name);
}

/// Set space `id`'s emoji icon.
pub fn set_space_icon(id: i32, icon: &str) {
    tabs::set_space_icon(id, icon);
}

/// Delete space `id` and close its tabs (refused for the last space).
pub fn delete_space(id: i32) {
    tabs::delete_space(id);
}

/// Promote folder `id` to its own space (its tabs move with it); returns the new
/// space id.
pub fn folder_to_space(id: i32) -> i32 {
    tabs::folder_to_space(id)
}

/// Demote space `id` to a folder in the active space (its pinned tabs move in;
/// its ephemeral tabs are closed); returns the new folder id.
pub fn space_to_folder(id: i32) -> i32 {
    tabs::space_to_folder(id)
}

/// Hand the run loop to CEF. Blocks until [`quit`] / all browsers close.
pub fn run_message_loop() {
    cef_run_message_loop();
}

/// Ask CEF to exit its message loop.
pub fn quit() {
    quit_message_loop();
}

/// The active tab's browser handle, if any. All the single-target verbs below
/// (navigate / back / reload / zoom / find / DevTools) act on the active tab;
/// the tab model owns which tab that is.
fn main_browser() -> Option<Browser> {
    tabs::active_browser()
}

/// Load `url` in the primary browser's main frame. Main thread only.
pub fn navigate(url: &str) {
    let Some(browser) = main_browser() else {
        return;
    };
    let Some(frame) = browser.main_frame() else {
        return;
    };
    frame.load_url(Some(&CefString::from(url)));
}

/// Navigate the primary browser back one history entry. Main thread only.
pub fn go_back() {
    if let Some(browser) = main_browser() {
        browser.go_back();
    }
}

/// Navigate the primary browser forward one history entry. Main thread only.
pub fn go_forward() {
    if let Some(browser) = main_browser() {
        browser.go_forward();
    }
}

/// Reload the primary browser's current page. Main thread only.
pub fn reload() {
    if let Some(browser) = main_browser() {
        browser.reload();
    }
}

/// Reload ignoring the cache (hard reload). Main thread only.
pub fn reload_ignore_cache() {
    if let Some(browser) = main_browser() {
        browser.reload_ignore_cache();
    }
}

/// Stop the primary browser's in-flight load — the reload button's cancel state.
/// Main thread only.
pub fn stop_load() {
    if let Some(browser) = main_browser() {
        browser.stop_load();
    }
}

/// Whether the primary browser can navigate back. Main thread only.
pub fn can_go_back() -> bool {
    main_browser().is_some_and(|browser| browser.can_go_back() != 0)
}

/// Whether the primary browser can navigate forward. Main thread only.
pub fn can_go_forward() -> bool {
    main_browser().is_some_and(|browser| browser.can_go_forward() != 0)
}

/// The primary browser's current main-frame URL, if a browser is alive. Used by
/// the host to prefill the command bar for edit-in-place (⌘L). Main thread only.
pub fn current_url() -> Option<String> {
    let browser = main_browser()?;
    let frame = browser.main_frame()?;
    Some(CefString::from(&frame.url()).to_string())
}

// ─── Adblock, targeting the active tab (ADR-0020) ────────────────────────────
//
// The sidebar shield and the "this site" toggle act on the active tab — the one
// whose URL the sidebar chip already shows — so these resolve the tab here
// rather than translating core tab ids to CEF ids across the FFI. Main thread.

/// The active tab's host, for per-site allowlist matching (registrable-domain
/// resolution happens in [`crate::adblock`]).
fn active_host() -> Option<String> {
    crate::vault::lookup::host_from_uri(&current_url()?)
}

/// Requests blocked on the active tab since its last navigation — the shield count.
pub fn adblock_active_blocked_count() -> u64 {
    main_browser().map_or(0, |browser| {
        crate::adblock::blocked_count(browser.identifier())
    })
}

/// Is ad-blocking turned **off** for the active tab's site?
pub fn adblock_active_site_allowlisted() -> bool {
    active_host().is_some_and(|host| crate::adblock::is_site_allowlisted(&host))
}

/// Turn ad-blocking off/on for the active tab's site, then reload so the change
/// applies to the page already on screen (matching Brave/Arc).
pub fn adblock_set_active_site_allowlisted(allowlisted: bool) {
    if let Some(host) = active_host() {
        crate::adblock::set_site_allowlisted(&host, allowlisted);
        reload();
    }
}

/// Toggle blocking globally, then reload the active tab so it applies on screen.
pub fn adblock_set_enabled(enabled: bool) {
    crate::adblock::set_enabled(enabled);
    reload();
}

/// Toggle cookie-consent handling (reject + hide), then reload the active tab so
/// the change applies on screen (ADR-0026, P2-25).
pub fn consent_set_enabled(enabled: bool) {
    crate::consent::set_enabled(enabled);
    reload();
}

/// Step the primary browser's zoom in / out / back to default. CEF owns the zoom
/// stepping (matching Chromium), so the host just names the command. Main thread.
pub fn zoom_in() {
    zoom(ZoomCommand::IN);
}

pub fn zoom_out() {
    zoom(ZoomCommand::OUT);
}

pub fn zoom_reset() {
    zoom(ZoomCommand::RESET);
}

fn zoom(command: ZoomCommand) {
    if let Some(host) = main_browser().and_then(|browser| browser.host()) {
        host.zoom(command);
    }
}

/// Toggle DevTools for the active tab: open it in its own window, or close it if
/// already open. Backs the ⌘⌥I menu item; F12 is handled engine-side in the
/// keyboard handler. Both funnel through [`crate::client::toggle_dev_tools`] so
/// the shortcuts behave identically. DevTools opens as a separate window — inline
/// docking isn't feasible under CEF 149's Chrome bootstrap (ADR-0021). Main thread only.
pub fn toggle_dev_tools() {
    if let Some(browser) = main_browser() {
        crate::client::toggle_dev_tools(&browser);
    }
}

/// Find `search_text` in the primary browser's page. `find_next` distinguishes a
/// fresh search (`false`) from stepping to the next/previous match of the same
/// term (`true`); `forward` picks the direction. Results arrive asynchronously
/// via the registered [`FindResultCallback`]. Main thread only.
pub fn find(search_text: &str, forward: bool, match_case: bool, find_next: bool) {
    if let Some(host) = main_browser().and_then(|browser| browser.host()) {
        host.find(
            Some(&CefString::from(search_text)),
            forward as i32,
            match_case as i32,
            find_next as i32,
        );
    }
}

/// End the current find-in-page session, optionally clearing the match selection
/// on the page. Main thread only.
pub fn stop_finding(clear_selection: bool) {
    if let Some(host) = main_browser().and_then(|browser| browser.host()) {
        host.stop_finding(clear_selection as i32);
    }
}

/// Route a host window close through CEF's browser-close flow.
///
/// Returns `true` if a close was initiated — the host must veto its window
/// close and wait: CEF re-requests the window close once the browser allows
/// it, and `BrowserHandler::on_before_close` quits the message loop when the
/// last browser is gone. Returns `false` if closing is already underway (or
/// there are no browsers) — the host should let the window close.
///
/// On that final close the host must also release the `NSView` it handed to
/// [`create_browser`]: CEF completes browser destruction (and thus
/// `on_before_close`) only when its child view deallocates.
pub fn request_close_all() -> bool {
    // App quit begins here: freeze session persistence so the browsers CEF is
    // about to drain aren't recorded as user tab-closes (ADR-0013). The session
    // file already reflects the last mutation, so nothing else is needed.
    tabs::begin_shutdown();
    let Some(handler) = crate::client::BrowserHandler::instance() else {
        return false;
    };
    {
        let mut h = handler.lock().expect("Failed to lock handler");
        if h.is_closing() {
            return false;
        }
        // Empty state (zero tabs): there is no browser whose `on_before_close`
        // would quit the loop, so quit it directly and let the window close.
        if h.browser_count() == 0 {
            drop(h);
            quit();
            return false;
        }
        // Flag app-quit so the final `on_before_close` (once every browser has
        // drained) quits the message loop.
        h.mark_closing();
    }
    crate::client::BrowserHandler::close_all_browsers(&handler, true);
    true
}

/// Tear down CEF. Call after [`run_message_loop`] returns.
pub fn shutdown() {
    cef_shutdown();
}
