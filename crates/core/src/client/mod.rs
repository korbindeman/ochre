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
use std::collections::HashMap;
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
        let Some(icon_url) = icon_urls.and_then(favicon::first_icon_url) else {
            return;
        };
        let Some(host_obj) = browser.and_then(|b| b.host()) else {
            return;
        };
        let mut callback = favicon::FaviconDownloadCallback::new(id, host);
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

    fn on_load_error(
        &mut self,
        _browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        error_code: Errorcode,
        error_text: Option<&CefString>,
        failed_url: Option<&CefString>,
    ) {
        handlers::show_load_error_page(
            self.runtime_style,
            frame,
            error_code,
            error_text,
            failed_url,
        );
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
