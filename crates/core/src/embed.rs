//! Embedded mode - the native app owns the windows and the run loop; the core
//! hosts one `WKWebView` per tab inside a container `NSView` the app hands it
//! (ADR-0072). The host drives the lifecycle across the FFI boundary:
//!
//!   1. [`initialize`] - load state, start background services.
//!   2. [`create_tab`] / [`restore_tab`] - a web view inside a host view.
//!   3. [`run_message_loop`] - `NSApplication.run`.
//!   4. [`request_close_all`] / [`shutdown`] - persist and exit.

use std::ffi::c_void;
use std::sync::Mutex;

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;

use crate::engine::{self, Browser, Profile, Zoom};
use crate::private_window::{self, WindowId};
use crate::tabs::{self, EventCallback, TabId};
use crate::{mac, root_cache_dir};

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

/// Initialize the core for embedded hosting. Returns `false` on failure.
///
/// Must be called once, on the main thread, before the host creates any window.
pub fn initialize() -> bool {
    crate::crash_report::install();
    if MainThreadMarker::new().is_none() {
        eprintln!("[embed] initialize must run on the main thread");
        return false;
    }

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

    // Open the history/favicon store in the profile folder. A failure here is
    // non-fatal - the browser just runs without autocomplete suggestions.
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
    // "reject all", plus hide the long-tail banners - gated on its own toggle,
    // whose persisted state loads here before the first browser.
    crate::consent::load_persisted();
    crate::consent::init_lists();
    crate::consent::register();

    // Sample page theme-color for the viewport URL bar (Safari-style tinting).
    crate::theme_color::register();

    // Load the appearance preference (Light/Dark/System, ADR-0030) before the
    // first browser. It is *applied* once the host forwards the OS appearance
    // (`browser_theme_set_system_dark`) early in window setup, so the global
    // request context is themed before any tab hydrates.
    crate::theme::load_persisted();
    crate::settings::load_persisted();
    crate::search_engine::load_persisted();

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
    // Restored tabs are suspended - the host hydrates each one (creating its
    // web view via `restore_tab`) the first time it is shown. With no saved
    // session the app launches to the empty state.
    tabs::init(&format!("{}/session.json", root_cache_dir()));

    mac::setup_app_delegate();
    true
}

// ─── Tab commands ────────────────────────────────────────────────────────
//
// The native app drives the tab lifecycle over the FFI: it owns one `NSView`
// per tab and calls these. Events flow back the other way, over the callback
// registered with [`set_event_callback`] (ADR-0003).

/// Register (or clear, with `None`) the host's tab/space event callback.
pub fn set_event_callback(callback: Option<EventCallback>) {
    tabs::set_event_callback(callback);
    private_window::set_event_callback(callback);
}

/// Tell the core which window owns keyboard/nav commands (`0` = normal).
pub fn set_focused_window(window_id: WindowId) {
    private_window::set_focused_window(window_id);
    tabs::set_focused_little(None);
}

/// Create a tab's web view inside `parent_view`, register it in the tab model
/// (active space), and make it active. Returns the new tab's id, or a negative
/// value on failure. Main thread only, after [`initialize`].
///
/// # Safety
/// `parent_view` must be a valid `NSView` pointer that outlives the tab.
pub unsafe fn create_tab(parent_view: *mut c_void, width: i32, height: i32, url: &str) -> TabId {
    let Some(browser) = create_tab_browser(parent_view, width, height, url) else {
        return -1;
    };
    tabs::register_tab(browser, url)
}

/// Create a little-window tab (P1-45) rendering into `parent_view`. It joins
/// no Space; [`promote_little_tab`] later moves it into the main window.
/// Returns the tab id or `-1`.
///
/// # Safety
/// `parent_view` must be a valid `NSView` pointer that outlives the tab.
pub unsafe fn create_little_tab(
    parent_view: *mut c_void,
    width: i32,
    height: i32,
    url: &str,
) -> TabId {
    let Some(browser) = create_tab_browser(parent_view, width, height, url) else {
        return -1;
    };
    tabs::register_little_tab(browser, url)
}

/// Move little-window tab `id` into the main window as the active open tab.
pub fn promote_little_tab(id: TabId) -> bool {
    tabs::promote_little_tab(id)
}

/// Close little-window tab `id` and tear down its browser.
pub fn close_little_tab(id: TabId) {
    tabs::close_little_tab(id);
}

/// A little window became key (`Some`), or another window did (`None`):
/// single-target commands follow it.
pub fn set_focused_little(id: Option<TabId>) {
    private_window::set_focused_window(private_window::NORMAL_WINDOW_ID);
    tabs::set_focused_little(id);
}

/// Hydrate a suspended tab restored from the persisted session (ADR-0013):
/// create its web view inside `parent_view`, loading the tab's saved
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

/// Shared web-view creation for [`create_tab`] / [`restore_tab`].
///
/// # Safety
/// `parent_view` must be a valid `NSView` pointer that outlives the browser.
unsafe fn create_tab_browser(
    parent_view: *mut c_void,
    width: i32,
    height: i32,
    url: &str,
) -> Option<Browser> {
    engine::create(parent_view, width, height, url, Profile::Normal)
}

/// Make tab `id` the active tab (of its space). The host reacts to the resulting
/// `StructureChanged` by showing that tab's view. Main thread only.
pub fn select_tab(id: TabId) {
    tabs::select_tab(id);
}

/// Leave element fullscreen / picture-in-picture on the active tab, if any.
/// WebKit runs element fullscreen in its own window, so the host window never
/// has to follow. Main thread only.
pub fn exit_fullscreen() {
    if let Some(browser) = main_browser() {
        browser.exit_fullscreen();
    }
}

/// Give keyboard focus to the active tab's web view, if any.
///
/// Used by the host to deterministically return focus to the page after a
/// transient chrome overlay (the command bar) closes. Without this, focus-return
/// depends on AppKit restoring the right first responder - which fails when the
/// pre-overlay responder was something else (e.g. a click into the sidebar left
/// the bare `NSWindow` as responder), silently dropping keystrokes. Mirrors the
/// tab-switch focus path (`tabs::set_visible`). No-op in the empty state (no
/// active browser). Main thread only.
pub fn focus_active_tab() {
    if let Some(browser) = main_browser() {
        browser.focus();
    }
}

/// Fill the active tab's login form from vault login `id` - the button-driven
/// autofill path (P2-34, ADR-0029). The credential crosses into the page on this
/// **explicit gesture** (the user picked the entry in the URL-bar popover), so we
/// fetch it here and evaluate a self-contained, lenient fill into the main frame's
/// main world. No-op when the vault is locked / `id` is unknown, or there is no
/// active tab. Main thread only.
pub fn vault_fill_active_tab(id: &str) {
    if private_window::focused_is_private() && !crate::settings::private_vault_enabled() {
        return;
    }
    let detail = match crate::vault::service::login_detail(id) {
        Some(detail) => detail,
        None => return,
    };
    let password = match detail.password {
        Some(password) => password,
        None => return,
    };
    let browser = match main_browser() {
        Some(browser) => browser,
        None => return,
    };
    let expr = crate::vault::autofill::fill_expression(detail.username.as_deref(), &password);
    crate::inject::evaluate_main_world(&browser, &expr);
    // The credential is now in the page - that's a use, so it rides to the top of
    // this site's list next time (the service applies the private-window rule).
    crate::vault::service::note_used(id);
}

/// Hide an active saved tab without removing it from the saved section. Main
/// thread only.
pub fn hide_saved_tab(id: TabId) {
    tabs::hide_saved_tab(id);
}

/// Permanently delete tab `id` (context-menu path). Main thread only.
pub fn delete_tab(id: TabId) {
    tabs::delete_tab(id);
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

/// Whether there is a closed tab to reopen - backs the menu item's enabled
/// state. Main thread only.
pub fn can_reopen_closed_tab() -> bool {
    tabs::can_reopen_closed()
}

/// Pin (persist) or unpin tab `id`. Unpinning returns it to "today".
pub fn set_tab_pinned(id: TabId, pinned: bool) {
    tabs::set_pinned(id, pinned);
}

/// Pin or unpin tab `id` in the Arc-style icon strip (ADR-0032).
pub fn set_tab_favorite(id: TabId, favorite: bool) {
    tabs::set_favorite(id, favorite);
}

/// Expand or collapse a Space's Pinned section. Main thread only.
pub fn set_space_pinned_expanded(id: i32, expanded: bool) {
    tabs::set_space_pinned_expanded(id, expanded);
}

/// Replace a saved tab/favorite's pinned URL with its current live URL.
pub fn replace_tab_pinned_url(id: TabId) {
    tabs::replace_pinned_url(id);
}

/// Move tab `id` into folder `folder` (negative ⇒ out of any folder). Moving
/// into a folder auto-pins the tab.
pub fn move_tab_to_folder(id: TabId, folder: i32) {
    tabs::move_to_folder(id, (folder >= 0).then_some(folder));
}

/// Drag-and-drop relocation: move tab `id` into a destination section - the
/// favorite strip (`favorite`), pinned/loose (`pinned`), or a folder (`folder`,
/// negative ⇒ no folder) - positioned just before tab `before` (negative ⇒ the
/// section's end). A folder destination auto-pins. One structural change; main
/// thread only.
pub fn move_tab(id: TabId, favorite: bool, pinned: bool, folder: i32, before: i32) {
    tabs::move_tab(
        id,
        favorite,
        pinned,
        (folder >= 0).then_some(folder),
        (before >= 0).then_some(before),
    );
}

/// Create an empty folder (named `name`) in the active space; returns its id.
pub fn create_folder(name: &str) -> i32 {
    tabs::create_folder(name)
}

/// Create a folder in the active space and move `tab_ids` into it (auto-pins).
pub fn create_folder_with_tabs(name: &str, tab_ids: &[i32]) -> i32 {
    tabs::create_folder_with_tabs(name, tab_ids)
}

/// Close ephemeral tabs in the active space except those in `keep`. When `keep`
/// is empty, the active tab survives.
pub fn clear_ephemeral_tabs(keep: &[i32]) {
    tabs::clear_ephemeral_tabs(keep)
}

/// Delete folder `id`, keeping its tabs (dropped to loose pinned).
pub fn delete_folder(id: i32) {
    tabs::delete_folder(id);
}

/// Rename folder `id`.
pub fn rename_folder(id: i32, name: &str) {
    tabs::rename_folder(id, name);
}

/// Persist folder `id`'s sidebar expand/collapse state (ADR-0039).
pub fn set_folder_expanded(id: i32, expanded: bool) {
    tabs::set_folder_expanded(id, expanded);
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

/// Select a tab by 1-based sidebar index (`1`…`8`, `9` = last) in the active space.
pub fn select_tab_by_index(index: u8) -> bool {
    tabs::select_tab_by_index(index)
}

/// Select the next (`1`) or previous (`-1`) tab in sidebar order.
pub fn select_adjacent_tab(delta: i32) -> bool {
    tabs::select_adjacent_tab(delta)
}

/// Select the next (`1`) or previous (`-1`) space.
pub fn select_adjacent_space(delta: i32) -> bool {
    tabs::select_adjacent_space(delta)
}

/// Move the visible tab up (`-1`) or down (`1`) in sidebar order.
pub fn move_visible_tab(delta: i32) -> bool {
    tabs::move_visible_tab(delta)
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
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).run();
    }
}

/// End the app (the last window closed, or quit finished draining).
pub fn quit() {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).terminate(None);
    }
}

/// The active tab's browser handle, if any. All the single-target verbs below
/// (navigate / back / reload / zoom / find / DevTools) act on the active tab;
/// the tab model owns which tab that is.
fn main_browser() -> Option<Browser> {
    if private_window::focused_is_private() {
        return private_window::active_browser_for_focused();
    }
    tabs::active_browser()
}

/// Create a new private window profile. Returns its id (`>= 1`).
pub fn create_private_window() -> WindowId {
    private_window::create_window()
}

/// Create a tab in private window `window_id`. Returns the tab id or `-1`.
///
/// # Safety
/// `parent_view` must be a valid `NSView*` that outlives the tab.
pub unsafe fn create_private_tab(
    window_id: WindowId,
    parent_view: *mut c_void,
    width: i32,
    height: i32,
    url: &str,
) -> TabId {
    private_window::create_tab(window_id, parent_view, width, height, url)
}

pub fn select_private_tab(window_id: WindowId, id: TabId) {
    private_window::select_tab(window_id, id);
}

pub fn select_private_tab_by_index(window_id: WindowId, index: u8) -> bool {
    private_window::select_tab_by_index(window_id, index)
}

pub fn select_private_adjacent_tab(window_id: WindowId, delta: i32) -> bool {
    private_window::select_adjacent_tab(window_id, delta)
}

pub fn close_private_tab(window_id: WindowId, id: TabId) {
    private_window::close_tab(window_id, id);
}

pub fn private_structure_json(window_id: WindowId) -> String {
    serde_json::to_string(&private_window::snapshot(window_id)).unwrap_or_default()
}

pub fn destroy_private_window(window_id: WindowId) {
    private_window::destroy_window(window_id);
}

pub fn private_window_count() -> usize {
    private_window::window_count()
}

pub fn private_is_site_allowlisted(window_id: WindowId, host: &str) -> bool {
    private_window::is_site_allowlisted(window_id, host)
}

pub fn private_set_site_allowlisted(window_id: WindowId, host: &str, allowlisted: bool) {
    private_window::set_site_allowlisted(window_id, host, allowlisted);
}

/// The active tab's host in the focused window (for per-site shield state).
pub fn active_tab_host() -> String {
    if private_window::focused_is_private() {
        return private_window::active_tab_host(private_window::focused_window())
            .unwrap_or_default();
    }
    tabs::active_tab_host().unwrap_or_default()
}

/// Load `url` in the primary browser's main frame. Main thread only.
pub fn navigate(url: &str) {
    let Some(browser) = main_browser() else {
        // The visible tab may be suspended (no frame yet); hand the URL to the
        // store so hydration loads it instead of dropping the navigation.
        if !private_window::focused_is_private() {
            tabs::navigate_suspended_visible(url);
        }
        return;
    };
    browser.load_url(url);
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

/// Stop the primary browser's in-flight load - the reload button's cancel state.
/// Main thread only.
pub fn stop_load() {
    if let Some(browser) = main_browser() {
        browser.stop();
    }
}

/// Whether the primary browser can navigate back. Main thread only.
pub fn can_go_back() -> bool {
    main_browser().is_some_and(|browser| browser.can_go_back())
}

/// Whether the primary browser can navigate forward. Main thread only.
pub fn can_go_forward() -> bool {
    main_browser().is_some_and(|browser| browser.can_go_forward())
}

/// The primary browser's current main-frame URL, if a browser is alive. Used by
/// the host to prefill the command bar for edit-in-place (⌘L). Main thread only.
pub fn current_url() -> Option<String> {
    main_browser()?.url()
}

// ─── Adblock, targeting the active tab (ADR-0020) ────────────────────────────
//
// The sidebar shield and the "this site" toggle act on the active tab - the one
// whose URL the sidebar chip already shows - so these resolve the tab here
// rather than translating core tab ids to engine ids across the FFI. Main thread.

/// The active tab's host, for per-site allowlist matching (registrable-domain
/// resolution happens in [`crate::adblock`]).
fn active_host() -> Option<String> {
    crate::vault::lookup::host_from_uri(&current_url()?)
}

/// Requests blocked on the active tab since its last navigation - the shield count.
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

/// Consent banners auto-rejected on the active tab since its last navigation -
/// the shield popover's transparency line (ADR-0026).
pub fn consent_active_rejected_count() -> u64 {
    main_browser().map_or(0, |browser| {
        crate::consent::rejected_count(browser.identifier())
    })
}

/// Step the primary browser's zoom in / out / back to default (Chrome's
/// ladder). Main thread.
pub fn zoom_in() {
    zoom(Zoom::In);
}

pub fn zoom_out() {
    zoom(Zoom::Out);
}

pub fn zoom_reset() {
    zoom(Zoom::Reset);
}

fn zoom(command: Zoom) {
    if let Some(browser) = main_browser() {
        browser.zoom(command);
    }
}

/// Toggle Web Inspector for the active tab (⌘⌥I). Main thread only.
pub fn toggle_dev_tools() {
    if let Some(browser) = main_browser() {
        browser.toggle_inspector();
    }
}

/// Find `search_text` in the primary browser's page. `find_next` distinguishes a
/// fresh search (`false`) from stepping to the next/previous match of the same
/// term (`true`); `forward` picks the direction. Results arrive asynchronously
/// via the registered [`FindResultCallback`]. Main thread only.
pub fn find(search_text: &str, forward: bool, match_case: bool, find_next: bool) {
    if let Some(browser) = main_browser() {
        browser.find(search_text, forward, match_case, find_next);
    }
}

/// End the current find-in-page session, optionally clearing the match selection
/// on the page. Main thread only.
pub fn stop_finding(clear_selection: bool) {
    if let Some(browser) = main_browser() {
        browser.stop_finding(clear_selection);
    }
}

/// Close one window's web views. `window_id` `0` = the normal window (which
/// quits the app, see [`request_close_all`]); `>= 1` = a private window.
/// Returns `true` if the host must veto its window close and wait; WebKit views
/// close synchronously, so this is always `false`.
pub fn request_close_window(window_id: WindowId) -> bool {
    if window_id == private_window::NORMAL_WINDOW_ID {
        return request_close_all();
    }
    private_window::destroy_window(window_id);
    false
}

/// App quit (⌘Q / main-window close): freeze session persistence so the
/// teardown isn't recorded as user tab closes (ADR-0013), close everything,
/// and let the host terminate. Always returns `false` (nothing to wait for).
pub fn request_close_all() -> bool {
    tabs::begin_shutdown();
    for id in private_window::all_window_ids() {
        private_window::destroy_window(id);
    }
    engine::close_all();
    false
}

/// Default Arc `StorableSidebar.json` path on macOS, if it exists.
pub fn arc_sidebar_default_path() -> Option<String> {
    crate::import::default_arc_sidebar_path()
}

/// Preview an Arc sidebar import (counts only; does not mutate the tab store).
pub fn import_arc_preview(path: &str) -> Result<crate::import::ArcImportPreview, String> {
    crate::import::preview_arc_import(path)
}

/// Import an Arc sidebar. `replace` clears the existing session first. Main thread.
pub fn import_arc(path: &str, replace: bool) -> Result<crate::import::ApplyResult, String> {
    crate::import::import_arc_sidebar(path, replace)
}

/// Process teardown after [`run_message_loop`] returns. Nothing to release:
/// WebKit's processes exit with ours.
pub fn shutdown() {}
