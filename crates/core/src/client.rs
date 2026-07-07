//! Client — the CEF Client and handler implementations.
//!
//! This module provides:
//! - `BrowserClient`: the top-level Client with display, life span, and load handlers.
//! - `BrowserHandler`: the shared state (browser list, closing flag).
//! - Window and browser-view delegates for the Views framework.

use cef::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use crate::history;
use crate::tabs;

// ─── Platform helpers ────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
use crate::mac::{platform_show_window, platform_title_change};

#[cfg(not(target_os = "macos"))]
fn platform_show_window(_browser: Option<&mut Browser>) {
    todo!("Implement platform_show_window for non-macOS platforms");
}

#[cfg(not(target_os = "macos"))]
fn platform_title_change(_browser: Option<&mut Browser>, _title: Option<&CefString>) {
    todo!("Implement platform_title_change for non-macOS platforms");
}

// ─── Shared handler state ────────────────────────────────────────────────

static HANDLER_INSTANCE: OnceLock<Weak<Mutex<BrowserHandler>>> = OnceLock::new();

/// Shared state for the browser handler — tracks all live browser instances
/// and the closing flag.
pub struct BrowserHandler {
    runtime_style: RuntimeStyle,
    browser_list: Vec<Browser>,
    is_closing: bool,
    weak_self: Weak<Mutex<Self>>,
    /// Per-browser main-frame URL, tracked from `on_address_change` so title
    /// and favicon changes attribute to the right history row. Keyed by CEF
    /// browser identifier (*not* the core tab id — the `tabs::note_*` calls
    /// translate, ADR-0013); multi-tab means one "current page" per browser,
    /// not one globally.
    current_url: HashMap<i32, String>,
}

impl BrowserHandler {
    pub fn instance() -> Option<Arc<Mutex<Self>>> {
        HANDLER_INSTANCE.get().and_then(|weak| weak.upgrade())
    }

    pub fn new(runtime_style: RuntimeStyle) -> Arc<Mutex<Self>> {
        Arc::new_cyclic(|weak| {
            if let Err(instance) = HANDLER_INSTANCE.set(weak.clone()) {
                assert_eq!(instance.strong_count(), 0, "Replacing a viable instance");
            }
            Mutex::new(Self {
                runtime_style,
                browser_list: Vec::new(),
                is_closing: false,
                weak_self: weak.clone(),
                current_url: HashMap::new(),
            })
        })
    }

    /// Record the main-frame URL as the current page and log a history visit.
    /// Subframe navigations and off-scheme URLs are ignored (the latter by
    /// [`history::record_visit`]).
    fn on_address_change(
        &mut self,
        browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        url: Option<&CefString>,
    ) {
        if frame.map(|f| f.is_main() != 0) != Some(true) {
            return;
        }
        let Some(id) = browser.as_deref().map(|b| b.identifier()) else {
            return;
        };
        let Some(url) = url.map(CefString::to_string) else {
            return;
        };
        history::record_visit(&url);
        self.current_url.insert(id, url.clone());
        // Push the address change to the host's tab model (a no-op for non-tab
        // browsers, e.g. popups, which are never registered as tabs).
        tabs::note_url(id, &url);
    }

    fn on_title_change(&mut self, browser: Option<&mut Browser>, title: Option<&CefString>) {
        let id = browser.as_deref().map(|b| b.identifier());
        // Persist the title against this tab's current page before the
        // window-title plumbing (which the Views/Alloy paths need) consumes it.
        let title_str = title.map(CefString::to_string).unwrap_or_default();
        if let Some(url) = id.and_then(|id| self.current_url.get(&id)) {
            history::update_title(url, &title_str);
        }
        if let Some(id) = id {
            tabs::note_title(id, &title_str);
        }
        let mut browser = browser.cloned();
        if let Some(browser_view) = browser_view_get_for_browser(browser.as_mut()) {
            if let Some(window) = browser_view.window() {
                window.set_title(title);
            }
        } else if self.runtime_style == RuntimeStyle::ALLOY {
            platform_title_change(browser.as_mut(), title);
        }
    }

    /// Download the current page's favicon and cache it under the page's host.
    /// CEF resolves the icon URL(s) from `<link rel="icon">`/`/favicon.ico`;
    /// we take the first and let CEF decode it, then store the PNG.
    fn on_favicon_urlchange(
        &mut self,
        browser: Option<&mut Browser>,
        icon_urls: Option<&mut CefStringList>,
    ) {
        let Some(id) = browser.as_deref().map(|b| b.identifier()) else {
            return;
        };
        let Some(host) = self
            .current_url
            .get(&id)
            .and_then(|url| history::host_of(url))
        else {
            return;
        };
        let Some(icon_url) = icon_urls.and_then(|list| first_icon_url(list)) else {
            return;
        };
        let Some(host_obj) = browser.and_then(|b| b.host()) else {
            return;
        };
        let mut callback = FaviconDownloadCallback::new(id, host);
        host_obj.download_image(
            Some(&CefString::from(icon_url.as_str())),
            1,  // is_favicon
            32, // max_image_size (px) — one row-height icon
            0,  // bypass_cache
            Some(&mut callback),
        );
    }

    fn on_after_created(&mut self, browser: Option<&mut Browser>) {
        eprintln!("[client] on_after_created");
        let browser = browser.cloned().expect("Browser is None");
        self.browser_list.push(browser);
    }

    /// Let every browser close proceed. Unlike the old single-window logic, this
    /// no longer flags "app closing" when one browser remains — closing the last
    /// *tab* must leave the app alive on its empty state. The app-quit flag is
    /// set only by [`mark_closing`](Self::mark_closing) (from `request_close_all`).
    fn do_close(&mut self, _browser: Option<&mut Browser>) -> bool {
        false
    }

    fn on_before_close(&mut self, browser: Option<&mut Browser>) {
        let mut browser = browser.cloned().expect("Browser is None");
        let id = browser.identifier();
        if let Some(index) = self
            .browser_list
            .iter()
            .position(|elem| elem.is_same(Some(&mut browser)) != 0)
        {
            self.browser_list.remove(index);
        }
        // Drop this tab's history-attribution entry and tell the host (a no-op
        // for non-tab browsers like popups / DevTools).
        self.current_url.remove(&id);
        tabs::note_closed(id);
        // Quit only when the whole app is closing (window close / ⌘Q) and the
        // last browser is gone — *not* when the user merely closed the last tab.
        if self.is_closing && self.browser_list.is_empty() {
            quit_message_loop();
        }
    }

    /// Number of live CEF browsers (tabs + popups + DevTools).
    pub fn browser_count(&self) -> usize {
        self.browser_list.len()
    }

    /// Flag that the whole app is shutting down, so the final `on_before_close`
    /// quits the message loop. Set from `request_close_all` (window close / ⌘Q).
    pub fn mark_closing(&mut self) {
        self.is_closing = true;
    }

    pub fn show_main_window(&mut self) {
        let thread_id = ThreadId::UI;
        if currently_on(thread_id) == 0 {
            let this = self
                .weak_self
                .upgrade()
                .expect("Weak ref to handler is None");
            let mut task = ShowMainWindowTask::new(this);
            post_task(thread_id, Some(&mut task));
            return;
        }
        let Some(mut main_browser) = self.browser_list.first().cloned() else {
            return;
        };
        if let Some(browser_view) = browser_view_get_for_browser(Some(&mut main_browser)) {
            if let Some(window) = browser_view.window() {
                window.show();
            }
        } else if self.runtime_style == RuntimeStyle::ALLOY {
            platform_show_window(Some(&mut main_browser));
        }
    }

    /// Close every live browser. Associated function (not a method) on
    /// purpose: `close_browser` synchronously re-enters `do_close` /
    /// `on_before_close` on the UI thread, and those handlers lock this same
    /// mutex — so the caller must NOT hold the lock, and this function only
    /// takes it briefly to snapshot the browser list.
    pub fn close_all_browsers(this: &Arc<Mutex<Self>>, force_close: bool) {
        let thread_id = ThreadId::UI;
        if currently_on(thread_id) == 0 {
            let mut task = CloseAllBrowsersTask::new(this.clone(), force_close);
            post_task(thread_id, Some(&mut task));
            return;
        }
        let browsers = this
            .lock()
            .expect("Failed to lock handler")
            .browser_list
            .clone();
        for browser in browsers.iter() {
            let host = browser.host().expect("BrowserHost is None");
            host.close_browser(force_close.into());
        }
    }

    pub fn is_closing(&self) -> bool {
        self.is_closing
    }
}

// ─── Client ──────────────────────────────────────────────────────────────

wrap_client! {
    pub struct BrowserClient {
        inner: Arc<Mutex<BrowserHandler>>,
    }

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(BrowserDisplayHandler::new(self.inner.clone()))
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(BrowserLifeSpanHandler::new(self.inner.clone()))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(BrowserLoadHandler::new(self.inner.clone()))
        }

        fn find_handler(&self) -> Option<FindHandler> {
            Some(BrowserFindHandler::new())
        }

        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            Some(BrowserContextMenuHandler::new())
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(BrowserKeyboardHandler::new())
        }

        fn focus_handler(&self) -> Option<FocusHandler> {
            Some(BrowserFocusHandler::new())
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(BrowserRequestHandler::new())
        }
    }
}

// Forwards find-in-page results to the host's registered callback (embedded
// mode only; a no-op in standalone, where no callback is set). Stateless — it
// carries no handler state, only bridges CEF's `on_find_result` to the FFI.
wrap_find_handler! {
    struct BrowserFindHandler;

    impl FindHandler {
        fn on_find_result(
            &self,
            _browser: Option<&mut Browser>,
            _identifier: i32,
            count: i32,
            _selection_rect: Option<&Rect>,
            active_match_ordinal: i32,
            final_update: i32,
        ) {
            crate::embed::dispatch_find_result(count, active_match_ordinal, final_update != 0);
        }
    }
}

// ─── DevTools & context menu ─────────────────────────────────────────────

/// Custom context-menu command id for "Inspect Element". CEF reserves the
/// `MENU_ID_USER_FIRST..=MENU_ID_USER_LAST` range for application-defined items.
const MENU_ID_INSPECT_ELEMENT: i32 = sys::cef_menu_id_t::MENU_ID_USER_FIRST as i32;

/// Toggle DevTools for `browser`: close it if already open, otherwise open it in
/// its own window. Backs both the ⌘⌥I menu item (via `embed::toggle_dev_tools`)
/// and the F12 keyboard shortcut, so the two stay consistent.
///
/// DevTools opens as a **separate window**, not docked in-window: under CEF 149's
/// Chrome bootstrap a DevTools browser must be Chrome-style, but hosting one in a
/// child `NSView` forces Alloy style (which it rejects), and its own Chrome-managed
/// window ignores `CefWindowInfo` (so it can't be created hidden for view-adoption
/// either). Inline docking is therefore not feasible for our Alloy-style tabs — see
/// ADR-0021. UI thread only.
pub(crate) fn toggle_dev_tools(browser: &Browser) {
    let Some(host) = browser.host() else { return };
    if host.has_dev_tools() != 0 {
        host.close_dev_tools();
    } else {
        host.show_dev_tools(None, None, None, None);
    }
}

/// Open DevTools focused on the element at the given page coordinates — the
/// context-menu "Inspect Element" path. Unlike [`toggle_dev_tools`] this always
/// opens (or refocuses) DevTools; it never closes it. UI thread only.
fn inspect_element_at(browser: &Browser, x: i32, y: i32) {
    if let Some(host) = browser.host() {
        host.show_dev_tools(None, None, None, Some(&Point { x, y }));
    }
}

// Adds "Inspect Element" to CEF's default page context menu and routes it to
// DevTools. The default model already carries navigation / editing / view-source
// items; we only append the inspect entry CEF never includes on its own. The
// DevTools window itself is created with a null client, so it keeps CEF's own
// context menu rather than this one.
wrap_context_menu_handler! {
    struct BrowserContextMenuHandler;

    impl ContextMenuHandler {
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
        ) {
            let Some(model) = model else { return };
            // Only separate from existing items — skip a leading separator on the
            // rare empty model.
            if model.count() > 0 {
                model.add_separator();
            }
            model.add_item(
                MENU_ID_INSPECT_ELEMENT,
                Some(&CefString::from("Inspect Element")),
            );
        }

        fn on_context_menu_command(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            params: Option<&mut ContextMenuParams>,
            command_id: i32,
            _event_flags: EventFlags,
        ) -> i32 {
            if command_id != MENU_ID_INSPECT_ELEMENT {
                return 0; // not ours — let CEF run its default command
            }
            if let (Some(browser), Some(params)) = (browser, params) {
                inspect_element_at(browser, params.xcoord(), params.ycoord());
            }
            1
        }
    }
}

// F12 toggles DevTools, matching Chrome. ⌘⌥I is handled by the native menu,
// which consumes the key equivalent before it reaches CEF, so this handler only
// needs the un-menued F12. Stateless.
wrap_keyboard_handler! {
    struct BrowserKeyboardHandler;

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: *mut u8,
            _is_keyboard_shortcut: Option<&mut i32>,
        ) -> i32 {
            let Some(event) = event else { return 0 };
            // Act on the raw key-down (once per press) so the page never sees F12.
            // `windows_key_code` is CEF's cross-platform VK code; VK_F12 = 0x7B.
            if event.type_ == KeyEventType::RAWKEYDOWN && event.windows_key_code == 0x7B {
                if let Some(browser) = browser {
                    toggle_dev_tools(browser);
                }
                return 1; // handled — don't forward to the renderer
            }
            0
        }
    }
}

// P0-1 focus-routing observability. The one hard-to-prove seam in the whole
// project is whether keyboard focus routes correctly between CEF's web view and
// the native chrome (page ↔ command bar ↔ sidebar ↔ tab switch). A screenshot
// can't show it and an occluded window can't exercise it, so this handler turns
// every CEF focus transition into a logged fact: `on_got_focus` is ground truth
// that the page actually holds keyboard focus. Paired with the Swift-side
// first-responder logs, it makes the routing verifiable by running (and later,
// a standing regression the CEF security-bump treadmill can re-check).
wrap_focus_handler! {
    struct BrowserFocusHandler;

    impl FocusHandler {
        // The web view has taken keyboard focus. Seeing this after the command
        // bar closes / after a click into the page / after a tab switch is the
        // proof that keys will land in the page.
        fn on_got_focus(&self, browser: Option<&mut Browser>) {
            let id = browser.map_or(-1, |b| b.identifier());
            eprintln!("[focus] cef on_got_focus tab={id}");
        }

        // The web view is about to receive focus. `source` distinguishes a
        // page-driven focus (NAVIGATION, e.g. autofocus) from a host/system one
        // (SYSTEM, e.g. our `set_focus` or a click). Returning 0 never cancels —
        // this only observes.
        fn on_set_focus(&self, browser: Option<&mut Browser>, source: FocusSource) -> i32 {
            let id = browser.map_or(-1, |b| b.identifier());
            let source = if source == FocusSource::NAVIGATION {
                "navigation"
            } else {
                "system"
            };
            eprintln!("[focus] cef on_set_focus tab={id} source={source}");
            0
        }

        // The web view is giving up focus (e.g. Tab-ing out of the page). `next`
        // is the traversal direction. Lets us see the page → chrome hand-off.
        fn on_take_focus(&self, browser: Option<&mut Browser>, next: i32) {
            let id = browser.map_or(-1, |b| b.identifier());
            let dir = if next != 0 { "next" } else { "previous" };
            eprintln!("[focus] cef on_take_focus tab={id} to={dir}");
        }
    }
}

wrap_display_handler! {
    struct BrowserDisplayHandler {
        inner: Arc<Mutex<BrowserHandler>>,
    }

    impl DisplayHandler {
        fn on_address_change(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.on_address_change(browser, frame, url);
        }

        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.on_title_change(browser, title);
        }

        fn on_favicon_urlchange(
            &self,
            browser: Option<&mut Browser>,
            icon_urls: Option<&mut CefStringList>,
        ) {
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.on_favicon_urlchange(browser, icon_urls);
        }
    }
}

wrap_life_span_handler! {
    struct BrowserLifeSpanHandler {
        inner: Arc<Mutex<BrowserHandler>>,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            // Attach the content-script injection substrate (a no-op unless a
            // consumer is registered) before the browser loads its first document
            // (ADR-0017). Reborrow so `browser` is still handed to the handler.
            let mut browser = browser;
            if let Some(browser) = browser.as_deref_mut() {
                crate::inject::install_for_browser(browser);
            }
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.on_after_created(browser);
        }

        fn do_close(&self, browser: Option<&mut Browser>) -> i32 {
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.do_close(browser).into()
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            // Drop the tab's adblock counter (ADR-0020) and its injection-substrate
            // observer registration (ADR-0017) so neither map grows unbounded.
            if let Some(id) = browser.as_deref().map(|b| b.identifier()) {
                crate::adblock::forget_tab(id);
                crate::inject::uninstall_for_browser(id);
            }
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.on_before_close(browser);
        }
    }
}

wrap_load_handler! {
    struct BrowserLoadHandler {
        inner: Arc<Mutex<BrowserHandler>>,
    }

    impl LoadHandler {
        // Drives each tab's spinner and its back/forward enablement: this one
        // callback carries both, so forward all three to the host's tab model
        // (a no-op for non-tab browsers). The nav-state stream is what lets the
        // sidebar disable the arrows live; `browser_can_go_back/forward` remain
        // for on-demand menu validation.
        fn on_loading_state_change(
            &self,
            browser: Option<&mut Browser>,
            is_loading: i32,
            can_go_back: i32,
            can_go_forward: i32,
        ) {
            if let Some(id) = browser.as_deref().map(|b| b.identifier()) {
                // Reset the tab's adblock count at each navigation start so the
                // sidebar shield reflects the current page (ADR-0020).
                if is_loading != 0 {
                    crate::adblock::reset_count(id);
                }
                tabs::note_load_state(id, is_loading != 0, can_go_back != 0, can_go_forward != 0);
            }
        }

        fn on_load_end(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            http_status_code: i32,
        ) {
            eprintln!("[client] on_load_end status={http_status_code}");
            // Autofill no longer hooks here: it rides the injection substrate
            // (isolated world, ask-the-core, gesture-gated) via `crate::inject`
            // — see `crate::vault::autofill` (P2-15, ADR-0017). The v0 main-world,
            // password-baked-in `on_load_end` push is retired.
        }

        fn on_load_error(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.on_load_error(browser, frame, error_code, error_text, failed_url);
        }
    }
}

// ─── Adblock request-path filtering (ADR-0020, spec §11) ─────────────────────

/// Map a CEF resource type to the lowercase request-type string adblock-rust
/// expects (`cpt_match_type`); unmapped kinds fall through to `"other"`, which
/// the engine treats as an unconstrained type.
fn adblock_request_type(resource_type: ResourceType) -> &'static str {
    match resource_type {
        ResourceType::MAIN_FRAME => "document",
        ResourceType::SUB_FRAME => "sub_frame",
        ResourceType::STYLESHEET => "stylesheet",
        ResourceType::SCRIPT => "script",
        ResourceType::IMAGE | ResourceType::FAVICON => "image",
        ResourceType::FONT_RESOURCE => "font",
        ResourceType::OBJECT => "object",
        ResourceType::MEDIA => "media",
        ResourceType::XHR => "xmlhttprequest",
        ResourceType::PING => "ping",
        ResourceType::CSP_REPORT => "csp_report",
        _ => "other",
    }
}

// Registers the resource-request handler that carries the network filter. CEF
// calls `resource_request_handler` once per request to obtain it; the handler is
// stateless (the engine + state are process-global in `crate::adblock`), so a
// fresh one per request is fine.
wrap_request_handler! {
    struct BrowserRequestHandler;

    impl RequestHandler {
        fn resource_request_handler(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
            _is_navigation: ::std::os::raw::c_int,
            _is_download: ::std::os::raw::c_int,
            _request_initiator: Option<&CefString>,
            _disable_default_handling: Option<&mut ::std::os::raw::c_int>,
        ) -> Option<ResourceRequestHandler> {
            Some(BrowserResourceRequestHandler::new())
        }
    }
}

// The network filter itself. Runs on the CEF **IO thread**: cancels ad/tracker
// requests before they hit the network and records the block against the tab for
// the sidebar shield. Cosmetic (element-hiding) filtering is the P2-21 fast-
// follow on the document-start injection substrate — this is network blocking.
wrap_resource_request_handler! {
    struct BrowserResourceRequestHandler;

    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            let Some(request) = request else {
                return ReturnValue::CONTINUE;
            };

            // Never cancel a top-level document load — blocking it would blank
            // the tab. Adblock targets sub-resources.
            let resource_type = request.resource_type();
            if resource_type == ResourceType::MAIN_FRAME {
                return ReturnValue::CONTINUE;
            }

            let url = CefString::from(&request.url()).to_string();
            // The top-level document URL — adblock's first-party for third-party
            // detection, and the host the per-site allowlist keys on.
            let source_url = CefString::from(&request.first_party_for_cookies()).to_string();
            let first_party_host =
                crate::vault::lookup::host_from_uri(&source_url).unwrap_or_default();

            let blocked = crate::adblock::check(
                &url,
                &source_url,
                &first_party_host,
                adblock_request_type(resource_type),
            );

            if blocked {
                if let Some(browser) = browser {
                    crate::adblock::note_blocked(browser.identifier());
                }
                return ReturnValue::CANCEL;
            }
            ReturnValue::CONTINUE
        }
    }
}

impl BrowserHandler {
    fn on_load_error(
        &mut self,
        _browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        error_code: Errorcode,
        error_text: Option<&CefString>,
        failed_url: Option<&CefString>,
    ) {
        // Only show custom error pages in Alloy style.
        if self.runtime_style != RuntimeStyle::ALLOY {
            return;
        }

        let error_code = sys::cef_errorcode_t::from(error_code);
        if error_code == sys::cef_errorcode_t::ERR_ABORTED {
            return;
        }
        let error_code = error_code as i32;

        let frame = frame.expect("Frame is None");
        let error_text = error_text.map(CefString::to_string).unwrap_or_default();
        let failed_url = failed_url.map(CefString::to_string).unwrap_or_default();
        let data = format!(
            r#"<html><body bgcolor="white">
            <h2>Failed to load URL {failed_url} with error {error_text} ({error_code}).</h2>
            </body></html>"#
        );
        let uri = get_data_uri(data.as_bytes(), "text/html");
        let uri = CefString::from(uri.as_str());
        frame.load_url(Some(&uri));
    }
}

fn get_data_uri(data: &[u8], mime_type: &str) -> String {
    let data = CefString::from(&base64_encode(Some(data)));
    let uri = CefString::from(&uriencode(Some(&data), 0)).to_string();
    format!("data:{mime_type};base64,{uri}")
}

// ─── Favicon capture ─────────────────────────────────────────────────────

/// First entry of a CEF string list, read without consuming it (the list is
/// borrowed from CEF and freed after the callback returns). Mirrors the raw
/// `cef_string_list_*` access the `cef` crate uses in its own `Debug` impl.
fn first_icon_url(list: &CefStringList) -> Option<String> {
    let raw: *const sys::_cef_string_list_t = list.into();
    if raw.is_null() {
        return None;
    }
    let raw = raw as *mut sys::_cef_string_list_t;
    unsafe {
        if sys::cef_string_list_size(raw) == 0 {
            return None;
        }
        let mut value = std::mem::zeroed();
        (sys::cef_string_list_value(raw, 0, &mut value) != 0)
            .then(|| CefString::from(std::ptr::from_ref(&value)).to_string())
    }
}

// `cef_id` is the CEF browser identifier of the requesting tab (translated to
// a tab id by `tabs::note_favicon`).
wrap_download_image_callback! {
    struct FaviconDownloadCallback {
        cef_id: i32,
        host: String,
    }

    impl DownloadImageCallback {
        fn on_download_image_finished(
            &self,
            _image_url: Option<&CefString>,
            _http_status_code: i32,
            image: Option<&mut Image>,
        ) {
            let Some(image) = image else { return };
            if image.is_empty() != 0 {
                return;
            }
            // Encode CEF's decoded image to PNG at 1x with transparency.
            let Some(png) = image.as_png(1.0, 1, None, None) else {
                return;
            };
            let size = png.size();
            if size == 0 {
                return;
            }
            let mut buf = vec![0u8; size];
            let written = png.data(Some(&mut buf), 0);
            buf.truncate(written);
            if !buf.is_empty() {
                history::store_favicon(&self.host, &buf);
                // Now that the PNG is cached, tell the host to display it.
                tabs::note_favicon(self.cef_id, &self.host);
            }
        }
    }
}

// ─── Tasks ───────────────────────────────────────────────────────────────

wrap_task! {
    struct ShowMainWindowTask {
        inner: Arc<Mutex<BrowserHandler>>,
    }

    impl Task {
        fn execute(&self) {
            debug_assert_ne!(currently_on(ThreadId::UI), 0);
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.show_main_window();
        }
    }
}

wrap_task! {
    struct CloseAllBrowsersTask {
        inner: Arc<Mutex<BrowserHandler>>,
        force_close: bool,
    }

    impl Task {
        fn execute(&self) {
            debug_assert_ne!(currently_on(ThreadId::UI), 0);
            BrowserHandler::close_all_browsers(&self.inner, self.force_close);
        }
    }
}

// ─── Window & view delegates (Views framework) ───────────────────────────

wrap_window_delegate! {
    pub struct BrowserWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
        runtime_style: RuntimeStyle,
        initial_show_state: ShowState,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 1024, height: 768 }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let browser_view = self.browser_view.borrow();
            let (Some(window), Some(browser_view)) = (window, browser_view.as_ref()) else {
                return;
            };
            let mut view = View::from(browser_view);
            window.add_child_view(Some(&mut view));
            if self.initial_show_state != ShowState::HIDDEN {
                window.show();
            }
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            let mut bv = self.browser_view.borrow_mut();
            *bv = None;
        }

        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            let bv = self.browser_view.borrow();
            let bv = bv.as_ref().expect("BrowserView is None");
            if let Some(browser) = bv.browser() {
                let host = browser.host().expect("BrowserHost is None");
                host.try_close_browser()
            } else {
                1
            }
        }

        fn initial_show_state(&self, _window: Option<&mut Window>) -> ShowState {
            self.initial_show_state
        }

        fn window_runtime_style(&self) -> RuntimeStyle {
            self.runtime_style
        }
    }
}

wrap_browser_view_delegate! {
    pub struct BrowserBrowserViewDelegate {
        runtime_style: RuntimeStyle,
    }

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        fn on_popup_browser_view_created(
            &self,
            _browser_view: Option<&mut BrowserView>,
            popup_browser_view: Option<&mut BrowserView>,
            _is_devtools: i32,
        ) -> i32 {
            let mut window_delegate = BrowserWindowDelegate::new(
                RefCell::new(popup_browser_view.cloned()),
                self.runtime_style,
                ShowState::NORMAL,
            );
            window_create_top_level(Some(&mut window_delegate));
            1
        }

        fn browser_runtime_style(&self) -> RuntimeStyle {
            self.runtime_style
        }
    }
}

impl BrowserClient {
    /// Build a CEF client wired to a fresh handler for the given runtime style.
    ///
    /// `wrap_client!` generates the actual `new(inner) -> Client` constructor
    /// (which fills the hidden `cef_object` field and returns the ref-counted
    /// `cef::Client`), so this convenience just constructs the handler and
    /// delegates to it.
    pub fn create(runtime_style: RuntimeStyle) -> Client {
        let inner = BrowserHandler::new(runtime_style);
        Self::new(inner)
    }
}
