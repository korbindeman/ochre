//! One Objective-C object per web view that receives WebKit's callbacks:
//! navigation policy and progress, UI requests (popups, dialogs, file pickers,
//! camera/mic), KVO on title/URL/load state, and popup-window closes.
//!
//! Callbacks that touch the tab model are posted to a later main-loop turn
//! (see [`crate::main_thread`]); WebKit completion handlers are copied and
//! answered from there when the decision needs the model.

use std::ffi::c_void;

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSEventModifierFlags, NSWindowDelegate};
use objc2_foundation::{NSArray, NSError, NSNotification, NSString, NSURL};
use objc2_web_kit::{
    WKDownload, WKFrameInfo, WKMediaCaptureType, WKNavigation, WKNavigationAction,
    WKNavigationActionPolicy, WKNavigationDelegate, WKNavigationResponse,
    WKNavigationResponsePolicy, WKNavigationType, WKOpenPanelParameters, WKPermissionDecision,
    WKSecurityOrigin, WKUIDelegate, WKWebView, WKWebViewConfiguration, WKWindowFeatures,
};

pub(crate) struct Ivars {
    id: i32,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OchreWebDelegate"]
    #[ivars = Ivars]
    pub(crate) struct WebDelegate;

    unsafe impl NSObjectProtocol for WebDelegate {}

    unsafe impl WKNavigationDelegate for WebDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_action(
            &self,
            _web_view: &WKWebView,
            action: &WKNavigationAction,
            handler: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            self.decide_navigation_action(action, handler);
        }

        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_response(
            &self,
            _web_view: &WKWebView,
            response: &WKNavigationResponse,
            handler: &DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            let policy = if should_download(response) {
                WKNavigationResponsePolicy::Download
            } else {
                WKNavigationResponsePolicy::Allow
            };
            handler.call((policy,));
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn did_start(&self, _web_view: &WKWebView, _navigation: Option<&WKNavigation>) {
            let id = self.ivars().id;
            crate::adblock::reset_count(id);
            crate::consent::reset_count(id);
            super::schedule_sync(id);
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn did_commit(&self, _web_view: &WKWebView, _navigation: Option<&WKNavigation>) {
            super::schedule_sync(self.ivars().id);
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, web_view: &WKWebView, _navigation: Option<&WKNavigation>) {
            let id = self.ivars().id;
            super::schedule_sync(id);
            super::favicon::collect_from_page(id, web_view);
            let blocked = crate::adblock::blocked_count(id);
            if blocked > 0 {
                let host = unsafe { web_view.URL() }.and_then(|u| u.host()).map(|h| h.to_string()).unwrap_or_default();
                eprintln!("[adblock] {blocked} requests blocked on {host}");
            }
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional(&self, web_view: &WKWebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            super::schedule_sync(self.ivars().id);
            super::ui::show_error_page(web_view, error);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail(&self, _web_view: &WKWebView, _navigation: Option<&WKNavigation>, _error: &NSError) {
            super::schedule_sync(self.ivars().id);
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn process_terminated(&self, web_view: &WKWebView) {
            eprintln!("[engine] web content process ended for {}; reloading", self.ivars().id);
            unsafe { web_view.reload() };
        }

        #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
        fn action_became_download(&self, _web_view: &WKWebView, _action: &WKNavigationAction, download: &WKDownload) {
            super::ui::adopt_download(download);
        }

        #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
        fn response_became_download(&self, _web_view: &WKWebView, _response: &WKNavigationResponse, download: &WKDownload) {
            super::ui::adopt_download(download);
        }
    }

    unsafe impl WKUIDelegate for WebDelegate {
        #[unsafe(method(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            _web_view: &WKWebView,
            configuration: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            features: &WKWindowFeatures,
        ) -> *mut WKWebView {
            let id = self.ivars().id;
            let url = action_url(action);
            // SAFETY: plain property reads on WebKit objects, main thread.
            let (width, height) = unsafe { (features.width(), features.height()) };
            let sized = width.is_some() || height.is_some();
            if url.is_empty() || url == "about:blank" || sized {
                let w = width.map(|n| n.as_f64()).unwrap_or(500.0);
                let h = height.map(|n| n.as_f64()).unwrap_or(600.0);
                return super::create_popup(id, configuration, w, h)
                    .map_or(std::ptr::null_mut(), Retained::autorelease_return);
            }
            let flags = unsafe { action.modifierFlags() };
            let background = flags.contains(NSEventModifierFlags::Command) && !flags.contains(NSEventModifierFlags::Shift);
            super::open_in_new_tab(id, url, !background);
            std::ptr::null_mut()
        }

        #[unsafe(method(webViewDidClose:))]
        fn did_close(&self, _web_view: &WKWebView) {
            super::page_closed(self.ivars().id);
        }

        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn alert(&self, web_view: &WKWebView, message: &NSString, frame: &WKFrameInfo, handler: &DynBlock<dyn Fn()>) {
            super::ui::alert(web_view, &frame_host(frame), &message.to_string(), handler);
        }

        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn confirm(&self, web_view: &WKWebView, message: &NSString, frame: &WKFrameInfo, handler: &DynBlock<dyn Fn(objc2::runtime::Bool)>) {
            super::ui::confirm(web_view, &frame_host(frame), &message.to_string(), handler);
        }

        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn prompt(
            &self,
            web_view: &WKWebView,
            prompt: &NSString,
            default_text: Option<&NSString>,
            frame: &WKFrameInfo,
            handler: &DynBlock<dyn Fn(*mut NSString)>,
        ) {
            let default_text = default_text.map(|s| s.to_string()).unwrap_or_default();
            super::ui::prompt(web_view, &frame_host(frame), &prompt.to_string(), &default_text, handler);
        }

        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn open_panel(
            &self,
            web_view: &WKWebView,
            parameters: &WKOpenPanelParameters,
            _frame: &WKFrameInfo,
            handler: &DynBlock<dyn Fn(*mut NSArray<NSURL>)>,
        ) {
            super::ui::open_panel(web_view, parameters, handler);
        }

        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn media_capture(
            &self,
            _web_view: &WKWebView,
            _origin: &WKSecurityOrigin,
            _frame: &WKFrameInfo,
            _kind: WKMediaCaptureType,
            handler: &DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            // WebKit's own per-site prompt, like Safari.
            handler.call((WKPermissionDecision::Prompt,));
        }
    }

    unsafe impl NSWindowDelegate for WebDelegate {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            let id = self.ivars().id;
            crate::main_thread::post_local(move || super::close_entry(id));
        }
    }

    impl WebDelegate {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(&self, _key_path: Option<&NSString>, _object: Option<&AnyObject>, _change: Option<&AnyObject>, _context: *mut c_void) {
            super::schedule_sync(self.ivars().id);
        }

        /// WebKit SPI (`WKNavigationDelegatePrivate`): a content rule acted on a
        /// load. Counts blocks for the sidebar shield; never called on systems
        /// without the SPI, where the count stays 0.
        #[unsafe(method(_webView:contentRuleListWithIdentifier:performedAction:forURL:))]
        fn rule_performed(&self, _web_view: &WKWebView, _identifier: &NSString, action: &AnyObject, _url: &NSURL) {
            let blocked: bool = unsafe { msg_send![action, blockedLoad] };
            if blocked {
                crate::adblock::note_blocked(self.ivars().id);
            }
        }
    }
);

impl WebDelegate {
    pub(super) fn new(mtm: MainThreadMarker, id: i32) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { id });
        // SAFETY: NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    }

    fn decide_navigation_action(
        &self,
        action: &WKNavigationAction,
        handler: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
    ) {
        let id = self.ivars().id;
        // SAFETY: property reads on the action, main thread.
        let (download, main_frame, link, flags) = unsafe {
            (
                action.shouldPerformDownload(),
                action
                    .targetFrame()
                    .map(|f| f.isMainFrame())
                    .unwrap_or(false),
                action.navigationType() == WKNavigationType::LinkActivated,
                action.modifierFlags(),
            )
        };
        if download {
            handler.call((WKNavigationActionPolicy::Download,));
            return;
        }
        let url = action_url(action);
        if hands_off_to_system(&url) {
            handler.call((WKNavigationActionPolicy::Cancel,));
            super::ui::open_externally(&url);
            return;
        }
        if link && flags.contains(NSEventModifierFlags::Command) {
            handler.call((WKNavigationActionPolicy::Cancel,));
            super::open_in_new_tab(id, url, flags.contains(NSEventModifierFlags::Shift));
            return;
        }
        if !(main_frame && link) {
            handler.call((WKNavigationActionPolicy::Allow,));
            return;
        }
        // A favorite's link that leaves its site opens in a little window
        // (P1-45). The check reads the tab model, so answer on a later turn.
        let handler = handler.copy();
        crate::main_thread::post_local(move || {
            if crate::tabs::favorite_link_leaves_site(id, &url, true) {
                handler.call((WKNavigationActionPolicy::Cancel,));
                crate::tabs::request_open_little(&url);
            } else {
                handler.call((WKNavigationActionPolicy::Allow,));
            }
        });
    }
}

fn action_url(action: &WKNavigationAction) -> String {
    // SAFETY: property reads, main thread.
    unsafe { action.request().URL() }
        .and_then(|u| u.absoluteString())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Schemes WebKit cannot show (mailto:, tel:, app links) go to the system.
fn hands_off_to_system(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    let scheme = scheme.to_ascii_lowercase();
    !matches!(
        scheme.as_str(),
        "http"
            | "https"
            | "about"
            | "blob"
            | "data"
            | "file"
            | "javascript"
            | "webkit-pdfjs-viewer"
    ) && MainThreadMarker::new()
        .map(|mtm| !unsafe { WKWebView::handlesURLScheme(&NSString::from_str(&scheme), mtm) })
        .unwrap_or(false)
}

/// Download instead of showing: types WebKit cannot display, and main-frame
/// responses that ask to be saved (`Content-Disposition: attachment`).
fn should_download(response: &WKNavigationResponse) -> bool {
    // SAFETY: property reads, main thread.
    unsafe {
        if !response.canShowMIMEType() {
            return true;
        }
        if !response.isForMainFrame() {
            return false;
        }
        let url_response = response.response();
        let is_http: bool =
            msg_send![&*url_response, isKindOfClass: objc2::class!(NSHTTPURLResponse)];
        if !is_http {
            return false;
        }
        let header: Option<Retained<NSString>> = msg_send![&*url_response, valueForHTTPHeaderField: &*NSString::from_str("Content-Disposition")];
        header.is_some_and(|h| {
            h.to_string()
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("attachment")
        })
    }
}

fn frame_host(frame: &WKFrameInfo) -> String {
    // SAFETY: property read, main thread.
    let origin = unsafe { frame.securityOrigin() };
    unsafe { origin.host() }.to_string()
}
