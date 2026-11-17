//! CEF handler trait implementations and the Views-framework delegates.

use cef::*;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use crate::tabs;

use super::BrowserHandler;

// Forwards find-in-page results to the host's registered callback (embedded
// mode only; a no-op in standalone, where no callback is set). Stateless - it
// carries no handler state, only bridges CEF's `on_find_result` to the FFI.
wrap_find_handler! {
    pub struct BrowserFindHandler;

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

// P0-1 focus-routing observability. The one hard-to-prove seam in the whole
// project is whether keyboard focus routes correctly between CEF's web view and
// the native chrome (page ↔ command bar ↔ sidebar ↔ tab switch). A screenshot
// can't show it and an occluded window can't exercise it, so this handler turns
// every CEF focus transition into a logged fact: `on_got_focus` is ground truth
// that the page actually holds keyboard focus. Paired with the Swift-side
// first-responder logs, it makes the routing verifiable by running (and later,
// a standing regression the CEF security-bump treadmill can re-check).
wrap_focus_handler! {
    pub struct BrowserFocusHandler;

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
        // (SYSTEM, e.g. our `set_focus` or a click). Returning 0 never cancels -
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
    pub struct BrowserDisplayHandler {
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

        // Alloy style: content already resizes to the browser area; the host must
        // enter/exit real OS window fullscreen (and hide chrome). Without this,
        // video "fullscreen" only fills the content pane inside the normal window.
        fn on_fullscreen_mode_change(
            &self,
            browser: Option<&mut Browser>,
            fullscreen: ::std::os::raw::c_int,
        ) {
            let cef_id = browser.map(|b| b.identifier()).unwrap_or(-1);
            let enter = fullscreen != 0;
            eprintln!("[fullscreen] content fullscreen={enter} cef={cef_id}");
            if !crate::private_window::note_fullscreen(cef_id, enter) {
                tabs::note_fullscreen(enter);
            }
        }
    }
}

/// Whether a window-open request should become an app tab instead of a bare CEF
/// popup. `target="_blank"` and ordinary new-tab dispositions route here;
/// same-tab navigations are left to CEF.
fn should_open_as_tab(disposition: WindowOpenDisposition) -> bool {
    !matches!(
        disposition,
        WindowOpenDisposition::CURRENT_TAB
            | WindowOpenDisposition::SINGLETON_TAB
            | WindowOpenDisposition::SWITCH_TO_TAB
            | WindowOpenDisposition::SAVE_TO_DISK
            | WindowOpenDisposition::OFF_THE_RECORD
            | WindowOpenDisposition::IGNORE_ACTION
            | WindowOpenDisposition::NEW_PICTURE_IN_PICTURE
            | WindowOpenDisposition::NEW_SPLIT_VIEW
    )
}

/// Whether a window-open request must stay a real CEF popup window.
///
/// OAuth and similar flows rely on `window.opener` and `window.close()` on a
/// script-opened popup - routing those to app tabs breaks login (the callback
/// never reaches the opener). Defer when:
/// - the URL is blank (the `window.open(''); w.location = url` pattern),
/// - Chromium marked it `NEW_POPUP`, or
/// - the page supplied explicit popup dimensions (a sized popup, not a tab link).
fn should_defer_popup_to_cef(
    url: &str,
    disposition: WindowOpenDisposition,
    popup_features: Option<&PopupFeatures>,
) -> bool {
    if url.is_empty() || url == crate::inject::BLANK_URL {
        return true;
    }
    if disposition == WindowOpenDisposition::NEW_POPUP {
        return true;
    }
    if let Some(features) = popup_features {
        if features.width_set != 0 && features.height_set != 0 {
            return true;
        }
    }
    false
}

/// Alloy popup `WindowInfo` for the rare opens we intentionally leave to CEF.
fn configure_popup_window_info(
    window_info: Option<&mut WindowInfo>,
    popup_features: Option<&PopupFeatures>,
) {
    let Some(info) = window_info else {
        return;
    };
    info.runtime_style = RuntimeStyle::ALLOY;
    let (x, y, width, height) = match popup_features {
        Some(features) if features.width_set != 0 && features.height_set != 0 => (
            if features.x_set != 0 { features.x } else { 100 },
            if features.y_set != 0 { features.y } else { 100 },
            features.width.max(100),
            features.height.max(100),
        ),
        _ => (100, 100, 500, 600),
    };
    info.bounds = Rect {
        x,
        y,
        width,
        height,
    };
}

wrap_task! {
    struct OpenTabTask {
        url: String,
        foreground: bool,
        source_cef_id: i32,
    }

    impl Task {
        fn execute(&self) {
            if crate::private_window::is_private_cef(self.source_cef_id) {
                crate::private_window::request_open_tab(
                    self.source_cef_id,
                    &self.url,
                    self.foreground,
                );
            } else {
                tabs::request_open_tab(&self.url, self.foreground);
            }
        }
    }
}

wrap_life_span_handler! {
    pub struct BrowserLifeSpanHandler {
        inner: Arc<Mutex<BrowserHandler>>,
    }

    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: i32,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            _user_gesture: i32,
            popup_features: Option<&PopupFeatures>,
            window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            if !should_open_as_tab(target_disposition) {
                return 0;
            }
            let url = target_url.map(CefString::to_string).unwrap_or_default();
            if should_defer_popup_to_cef(&url, target_disposition, popup_features) {
                configure_popup_window_info(window_info, popup_features);
                return 0;
            }
            let foreground = target_disposition != WindowOpenDisposition::NEW_BACKGROUND_TAB;
            let source_cef_id = browser.as_deref().map(|b| b.identifier()).unwrap_or(-1);
            let mut task = OpenTabTask::new(url, foreground, source_cef_id);
            post_task(ThreadId::UI, Some(&mut task));
            1
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            // Attach the content-script injection substrate (a no-op unless a
            // consumer is registered) before the browser loads its first document
            // (ADR-0017). Reborrow so `browser` is still handed to the handler.
            let mut browser = browser;
            if let Some(browser) = browser.as_deref_mut() {
                let id = browser.identifier();
                // Claim before install: `install_for_browser` returns early when no
                // inject consumers are registered, but staged first navigations must
                // still attach (the about:blank hop path, if used).
                crate::inject::claim_staged_navigation(id);
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
            // Drop the tab's adblock + consent counters (ADR-0020, ADR-0026) and its
            // injection-substrate observer registration (ADR-0017) so none of the
            // maps grow unbounded.
            if let Some(id) = browser.as_deref().map(|b| b.identifier()) {
                crate::adblock::forget_tab(id);
                crate::consent::forget_tab(id);
                crate::inject::uninstall_for_browser(id);
            }
            let mut inner = self.inner.lock().expect("Failed to lock handler");
            inner.on_before_close(browser);
        }
    }
}

wrap_load_handler! {
    pub struct BrowserLoadHandler {
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
                // Reset the tab's adblock + consent counts at each navigation start so
                // the shield reflects the current page (ADR-0020, ADR-0026).
                if is_loading != 0 {
                    crate::adblock::reset_count(id);
                    crate::consent::reset_count(id);
                }
                if !crate::private_window::note_load_state(
                    id,
                    is_loading != 0,
                    can_go_back != 0,
                    can_go_forward != 0,
                ) {
                    tabs::note_load_state(id, is_loading != 0, can_go_back != 0, can_go_forward != 0);
                }
            }
        }

        fn on_load_end(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            http_status_code: i32,
        ) {
            eprintln!("[client] on_load_end status={http_status_code}");
            if let (Some(browser), Some(frame)) = (browser, frame) {
                crate::inject::dispatch_pending_navigation(browser, frame);
                crate::inject::bootstrap_on_load_end(browser, frame);
                if frame.is_main() != 0 {
                    let cef_id = browser.identifier();
                    let inner = self.inner.lock().expect("Failed to lock handler");
                    inner.on_load_end_favicon(cef_id, http_status_code);
                }
            }
            // Autofill no longer hooks here: it rides the injection substrate
            // (isolated world, ask-the-core, gesture-gated) via `crate::inject`
            // - see `crate::vault::autofill` (P2-15, ADR-0017). The v0 main-world,
            // password-baked-in `on_load_end` push is retired.
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            // Chromium renders its own net-error page in ALLOY. Do not call
            // `frame.load_url` here - a custom data-URI page flashes over the
            // native one, and loading while holding `BrowserHandler`'s mutex
            // deadlocks the UI thread.
            if frame.as_ref().map(|f| f.is_main() != 0) != Some(true) {
                return;
            }
            let code = sys::cef_errorcode_t::from(error_code);
            if code == sys::cef_errorcode_t::ERR_ABORTED {
                return;
            }
            let error_text = error_text.map(CefString::to_string).unwrap_or_default();
            let failed_url = failed_url.map(CefString::to_string).unwrap_or_default();
            eprintln!(
                "[client] on_load_error code={code:?} url={failed_url} {error_text}"
            );
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
    pub struct BrowserRequestHandler;

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
// follow on the document-start injection substrate - this is network blocking.
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

            // Never cancel a top-level document load - blocking it would blank
            // the tab. Adblock targets sub-resources.
            let resource_type = request.resource_type();
            if resource_type == ResourceType::MAIN_FRAME {
                return ReturnValue::CONTINUE;
            }

            let url = CefString::from(&request.url()).to_string();
            // The top-level document URL - adblock's first-party for third-party
            // detection, and the host the per-site allowlist keys on.
            let source_url = CefString::from(&request.first_party_for_cookies()).to_string();
            let first_party_host =
                crate::vault::lookup::host_from_uri(&source_url).unwrap_or_default();

            let ephemeral = crate::private_window::ephemeral_allowlist_for_cef(
                browser.as_deref().map(|b| b.identifier()).unwrap_or(-1),
            );
            let blocked = crate::adblock::check(
                &url,
                &source_url,
                &first_party_host,
                adblock_request_type(resource_type),
                ephemeral.as_deref(),
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

#[cfg(test)]
mod tests {
    use cef::*;

    use super::{should_defer_popup_to_cef, should_open_as_tab};

    #[test]
    fn target_blank_dispositions_open_as_tab() {
        assert!(should_open_as_tab(
            WindowOpenDisposition::NEW_FOREGROUND_TAB
        ));
        assert!(should_open_as_tab(
            WindowOpenDisposition::NEW_BACKGROUND_TAB
        ));
        assert!(should_open_as_tab(WindowOpenDisposition::NEW_WINDOW));
        assert!(should_open_as_tab(WindowOpenDisposition::NEW_POPUP));
    }

    #[test]
    fn sized_popup_opens_as_tab() {
        assert!(should_open_as_tab(WindowOpenDisposition::NEW_POPUP));
    }

    #[test]
    fn blank_or_empty_urls_defer_to_cef_popup() {
        assert!(should_defer_popup_to_cef(
            "about:blank",
            WindowOpenDisposition::NEW_FOREGROUND_TAB,
            None,
        ));
        assert!(should_defer_popup_to_cef(
            "",
            WindowOpenDisposition::NEW_FOREGROUND_TAB,
            None,
        ));
    }

    #[test]
    fn new_popup_and_sized_opens_defer_to_cef_popup() {
        assert!(should_defer_popup_to_cef(
            "https://accounts.google.com/o/oauth2/auth",
            WindowOpenDisposition::NEW_POPUP,
            None,
        ));
        let features = PopupFeatures {
            width_set: 1,
            height_set: 1,
            width: 500,
            height: 600,
            ..Default::default()
        };
        assert!(should_defer_popup_to_cef(
            "https://accounts.google.com/o/oauth2/auth",
            WindowOpenDisposition::NEW_FOREGROUND_TAB,
            Some(&features),
        ));
        assert!(!should_defer_popup_to_cef(
            "https://example.com",
            WindowOpenDisposition::NEW_FOREGROUND_TAB,
            None,
        ));
    }

    #[test]
    fn same_tab_dispositions_stay_with_cef() {
        assert!(!should_open_as_tab(WindowOpenDisposition::CURRENT_TAB));
        assert!(!should_open_as_tab(WindowOpenDisposition::SWITCH_TO_TAB));
    }
}
