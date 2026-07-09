//! Client — the CEF Client and handler implementations.
//!
//! This module provides:
//! - `BrowserClient`: the top-level Client with display, life span, and load handlers.
//! - `BrowserHandler`: the shared state (browser list, closing flag).
//! - Window and browser-view delegates for the Views framework.

mod devtools;
mod favicon;
mod handlers;

use cef::*;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use crate::history;
use crate::tabs;

use devtools::{BrowserContextMenuHandler, BrowserKeyboardHandler};
use handlers::{
    BrowserDisplayHandler, BrowserFindHandler, BrowserFocusHandler, BrowserLifeSpanHandler,
    BrowserLoadHandler, BrowserRequestHandler,
};

pub(crate) use devtools::toggle_dev_tools;
pub use handlers::{BrowserBrowserViewDelegate, BrowserWindowDelegate};

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
    /// CEF ids of browsers whose view is embedded in a **host-owned** window
    /// (tabs, normal and private) — registered at creation, dropped in
    /// `on_before_close`. [`do_close`](Self::do_close) must return `true` for
    /// these: CEF's default close would `performClose:` the browser's
    /// top-level parent window, i.e. the whole app window (ADR-0037).
    embedded_tabs: HashSet<i32>,
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
                embedded_tabs: HashSet::new(),
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
        if !crate::private_window::is_private_cef(id) {
            history::record_visit(&url);
        }
        self.current_url.insert(id, url.clone());
        // Push the address change to the host's tab model (a no-op for non-tab
        // browsers, e.g. popups, which are never registered as tabs).
        if !crate::private_window::note_url(id, &url) {
            tabs::note_url(id, &url);
        }
    }

    fn on_title_change(&mut self, browser: Option<&mut Browser>, title: Option<&CefString>) {
        let id = browser.as_deref().map(|b| b.identifier());
        // Persist the title against this tab's current page before the
        // window-title plumbing (which the Views/Alloy paths need) consumes it.
        let title_str = title.map(CefString::to_string).unwrap_or_default();
        if let Some(cef_id) = id {
            if !crate::private_window::is_private_cef(cef_id) {
                if let Some(url) = self.current_url.get(&cef_id) {
                    history::update_title(url, &title_str);
                }
            }
            if !crate::private_window::note_title(cef_id, &title_str) {
                tabs::note_title(cef_id, &title_str);
            }
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
    /// we prefer raster formats (ICO/PNG) over SVG, then store the PNG.
    fn on_favicon_urlchange(
        &mut self,
        browser: Option<&mut Browser>,
        icon_urls: Option<&mut CefStringList>,
    ) {
        let Some(id) = browser.as_deref().map(|b| b.identifier()) else {
            return;
        };
        let Some(page_url) = self.current_url.get(&id).cloned() else {
            return;
        };
        let Some(host) = history::host_of(&page_url) else {
            return;
        };
        let candidates = match icon_urls {
            Some(list) => favicon::rank_icon_urls_from_list(list, &page_url, &host),
            None => favicon::rank_icon_urls(&[], &page_url, &host),
        };
        if candidates.is_empty() {
            return;
        }
        favicon::start_favicon_download(id, host, candidates);
    }

    /// Sites without declared `<link rel="icon">` tags never fire
    /// `on_favicon_urlchange`; probe well-known paths once the main frame loads.
    fn on_load_end_favicon(&self, cef_id: i32, http_status_code: i32) {
        if !(200..400).contains(&http_status_code) {
            return;
        }
        let Some(page_url) = self.current_url.get(&cef_id).cloned() else {
            return;
        };
        favicon::fetch_favicon_if_missing(cef_id, &page_url);
    }

    fn on_after_created(&mut self, browser: Option<&mut Browser>) {
        eprintln!("[client] on_after_created");
        let browser = browser.cloned().expect("Browser is None");
        self.browser_list.push(browser);
    }

    /// Decide who closes the window when a browser closes (ADR-0037).
    ///
    /// Returning `false` makes CEF send the platform close (`performClose:` on
    /// macOS) to the browser's **top-level parent window** — for a tab that is
    /// the host's own `NSWindow`, so a mere tab close/hide/suspend would close
    /// the whole window whenever the tab's view is still attached when CEF's
    /// close pipeline lands. The host's view detach usually wins that race,
    /// but nothing guarantees it (⌘W on an active saved tab lost it). So for
    /// host-embedded tabs return `true`: the host always detaches the tab's
    /// view, and that detach is what completes browser destruction.
    ///
    /// The default `false` remains for:
    /// - app shutdown (`is_closing`): CEF's close re-request is what lets the
    ///   host's vetoed `windowShouldClose` proceed (`embed::request_close_all`);
    /// - non-tab browsers (popups, DevTools): CEF created their window, and
    ///   the default close is what closes it — OAuth popups rely on
    ///   `window.close()` working this way.
    fn do_close(&mut self, browser: Option<&mut Browser>) -> bool {
        if self.is_closing {
            return false;
        }
        let Some(id) = browser.as_deref().map(|b| b.identifier()) else {
            return false;
        };
        eprintln!(
            "[client] do_close id={id} embedded={}",
            self.embedded_tabs.contains(&id)
        );
        if !self.embedded_tabs.contains(&id) {
            return false;
        }
        // A page that closed itself (`window.close()` in a tab) arrives here
        // with its tab still in the model: drop the tab row so the host prunes
        // the view, completing destruction. A no-op for host-initiated closes —
        // those already removed the tab or took its browser binding.
        if crate::private_window::is_private_cef(id) {
            crate::private_window::note_closed(id);
        } else {
            tabs::note_closed(id);
        }
        true
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
        self.embedded_tabs.remove(&id);
        if crate::private_window::is_private_cef(id) {
            crate::private_window::note_closed(id);
        } else {
            tabs::note_closed(id);
        }
        eprintln!(
            "[client] on_before_close id={id} remaining={} closing={}",
            self.browser_list.len(),
            self.is_closing
        );
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

    /// Snapshot browsers whose CEF id satisfies `pred`.
    pub(crate) fn matching_browsers(&self, pred: impl Fn(i32) -> bool) -> Vec<Browser> {
        self.browser_list
            .iter()
            .filter(|b| pred(b.identifier()))
            .cloned()
            .collect()
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

/// Register `browser` as host-embedded — a tab whose view lives inside a
/// host-owned window (ADR-0037). Called right after the synchronous tab
/// creation paths (`embed::create_tab_browser`, private windows); popups and
/// DevTools never pass through those, so they keep CEF's default close.
pub(crate) fn note_embedded_tab(browser: &Browser) {
    if let Some(handler) = BrowserHandler::instance() {
        handler
            .lock()
            .expect("Failed to lock handler")
            .embedded_tabs
            .insert(browser.identifier());
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

impl BrowserClient {
    /// Build a CEF client wired to **the** shared handler (creating it on first
    /// use). Every client must share one `BrowserHandler`: `browser_list`,
    /// `is_closing`, and `embedded_tabs` are app-global state, and everything
    /// that drains or counts browsers (`close_all_browsers`, `browser_count`,
    /// `mark_closing`) reaches them through [`BrowserHandler::instance`]. A
    /// fresh handler per client silently splits that state per tab — each
    /// browser's `do_close`/`on_before_close` then consults a handler the rest
    /// of the app never talks to.
    ///
    /// `wrap_client!` generates the actual `new(inner) -> Client` constructor
    /// (which fills the hidden `cef_object` field and returns the ref-counted
    /// `cef::Client`), so this convenience just resolves the handler and
    /// delegates to it.
    pub fn create(runtime_style: RuntimeStyle) -> Client {
        let inner =
            BrowserHandler::instance().unwrap_or_else(|| BrowserHandler::new(runtime_style));
        Self::new(inner)
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
