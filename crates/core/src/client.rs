//! Client — the CEF Client and handler implementations.
//!
//! This module provides:
//! - `BrowserClient`: the top-level Client with display, life span, and load handlers.
//! - `BrowserHandler`: the shared state (browser list, closing flag).
//! - Window and browser-view delegates for the Views framework.

use cef::*;
use std::sync::{Arc, Mutex, OnceLock, Weak};

// ─── Platform helpers ────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac::*;

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
            })
        })
    }

    fn on_title_change(&mut self, browser: Option<&mut Browser>, title: Option<&CefString>) {
        let mut browser = browser.cloned();
        if let Some(browser_view) = browser_view_get_for_browser(browser.as_mut()) {
            if let Some(window) = browser_view.window() {
                window.set_title(title);
            }
        } else if self.runtime_style == RuntimeStyle::ALLOY {
            platform_title_change(browser.as_mut(), title);
        }
    }

    fn on_after_created(&mut self, browser: Option<&mut Browser>) {
        let browser = browser.cloned().expect("Browser is None");
        self.browser_list.push(browser);
    }

    fn do_close(&mut self, _browser: Option<&mut Browser>) -> bool {
        if self.browser_list.len() == 1 {
            self.is_closing = true;
        }
        false
    }

    fn on_before_close(&mut self, browser: Option<&mut Browser>) {
        let mut browser = browser.cloned().expect("Browser is None");
        if let Some(index) = self
            .browser_list
            .iter()
            .position(|elem| elem.is_same(Some(&mut browser)) != 0)
        {
            self.browser_list.remove(index);
        }
        if self.browser_list.is_empty() {
            quit_message_loop();
        }
    }

    pub fn show_main_window(&mut self) {
        let thread_id = ThreadId::UI;
        if currently_on(thread_id) == 0 {
            let this = self.weak_self.upgrade().expect("Weak ref to handler is None");
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

    pub fn close_all_browsers(&mut self, force_close: bool) {
        let thread_id = ThreadId::UI;
        if currently_on(thread_id) == 0 {
            let this = self.weak_self.upgrade().expect("Weak ref to handler is None");
            let mut task = CloseAllBrowsersTask::new(this, force_close);
            post_task(thread_id, Some(&mut task));
            return;
        }
        for browser in self.browser_list.iter() {
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
    }
}

wrap_display_handler! {
    struct BrowserDisplayHandler {
        inner: Arc<Mutex<BrowserHandler>>,
    }

    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.on_title_change(browser, title);
        }
    }
}

wrap_life_span_handler! {
    struct BrowserLifeSpanHandler {
        inner: Arc<Mutex<BrowserHandler>>,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.on_after_created(browser);
        }

        fn do_close(&self, browser: Option<&mut Browser>) -> i32 {
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.do_close(browser).into()
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
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
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.close_all_browsers(self.force_close);
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
    pub fn new(runtime_style: RuntimeStyle) -> Self {
        let inner = BrowserHandler::new(runtime_style);
        Self { inner }
    }
}
