//! The web engine: WKWebView, driven from Rust (ADR-0072).
//!
//! The tab model holds a [`Browser`] per hydrated tab. It is a plain id, so it
//! is `Send` and can live inside the core's global stores; the WebKit
//! objects it names stay in a main-thread registry here. Every method is a
//! no-op off the main thread or once the browser is closed, the same tolerance
//! the old CEF handle had (`host()` returning `None`).
//!
//! Hosting model: the native frontend owns the window and hands the core a
//! container `NSView` per tab; the core inserts the `WKWebView` as its subview
//! (sized to it, autoresizing). Engine callbacks (title, URL, load state,
//! popups, closes) reach the tab model through [`crate::main_thread::post`],
//! never inline, because WebKit fires some of them inside our own calls.
//!
//! Profiles: the normal window uses WebKit's default persistent data store;
//! each private window gets its own non-persistent store, so its cookies and
//! storage vanish with the window. Each profile has one shared
//! `WKUserContentController` carrying the content scripts, the script-message
//! bridge and the compiled ad-block rules.

pub mod content_rules;
mod delegate;
pub mod favicon;
mod ui;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{msg_send, sel, MainThreadMarker, Message};
use objc2_app_kit::{NSAppearance, NSAppearanceCustomization, NSView, NSWindow};
use objc2_foundation::{NSNumber, NSPoint, NSRect, NSSize, NSString, NSURLRequest, NSURL};
use objc2_web_kit::{
    WKContentWorld, WKFindConfiguration, WKFindResult, WKUserContentController, WKWebView,
    WKWebViewConfiguration, WKWebsiteDataStore,
};

use crate::private_window::WindowId;
use delegate::WebDelegate;

/// A live web view, by id. Ids are unique for the process and never reused.
/// `Clone` (not `Copy`) to match how the tab model passes handles around.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Browser(i32);

/// Which data store and content controller a web view uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Profile {
    Normal,
    Private(WindowId),
}

struct ProfileCtx {
    store: Retained<WKWebsiteDataStore>,
    content: Retained<WKUserContentController>,
}

struct Entry {
    webview: Retained<WKWebView>,
    delegate: Retained<WebDelegate>,
    profile: Profile,
    /// Set for script-opened popups, which get their own window.
    popup_window: Option<Retained<NSWindow>>,
    /// Last state pushed to the core, so KVO bursts emit each change once.
    last: Cell<Option<PageState>>,
    sync_pending: Cell<bool>,
}

#[derive(Clone, PartialEq, Eq, Default)]
struct PageState {
    url: String,
    title: String,
    loading: bool,
    back: bool,
    forward: bool,
}

thread_local! {
    static VIEWS: RefCell<HashMap<i32, Entry>> = RefCell::new(HashMap::new());
    static PROFILES: RefCell<HashMap<Profile, ProfileCtx>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<i32> = const { Cell::new(1) };
    static DARK: Cell<Option<bool>> = const { Cell::new(None) };
    static FIND: RefCell<HashMap<i32, FindState>> = RefCell::new(HashMap::new());
}

struct FindState {
    text: String,
    ordinal: i32,
}

const OBSERVED_KEYS: [&str; 5] = ["title", "URL", "loading", "canGoBack", "canGoForward"];

/// Chrome's zoom ladder, so ⌘+ / ⌘- step the same way users expect.
const ZOOM_LEVELS: [f64; 15] = [
    0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];

#[derive(Clone, Copy, Debug)]
pub enum Zoom {
    In,
    Out,
    Reset,
}

fn mtm() -> Option<MainThreadMarker> {
    let mtm = MainThreadMarker::new();
    if mtm.is_none() {
        eprintln!("[engine] called off the main thread; ignored");
    }
    mtm
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

fn with_entry<R>(id: i32, f: impl FnOnce(&Entry) -> R) -> Option<R> {
    mtm()?;
    VIEWS.with(|views| views.borrow().get(&id).map(f))
}

fn with_view<R>(id: i32, f: impl FnOnce(&WKWebView) -> R) -> Option<R> {
    with_entry(id, |entry| f(&entry.webview))
}

fn next_id() -> i32 {
    NEXT_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    })
}

// ─── Profiles ────────────────────────────────────────────────────────────────

/// `Version/<safari> Safari/605.1.15`, appended to WebKit's own user agent so
/// sites see Safari. Without the `Safari/` token Google refuses sign-in as an
/// "embedded browser", and UA-sniffing sites serve degraded pages.
fn application_name_for_user_agent() -> String {
    let version = safari_version().unwrap_or_else(|| "26.0".to_string());
    format!("Version/{version} Safari/605.1.15")
}

fn safari_version() -> Option<String> {
    let plist = std::fs::read_to_string("/Applications/Safari.app/Contents/Info.plist")
        .or_else(|_| {
            std::fs::read_to_string(
                "/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app/Contents/Info.plist",
            )
        })
        .ok()?;
    let key = plist.find("<key>CFBundleShortVersionString</key>")?;
    let rest = &plist[key..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = rest[start..].find("</string>")?;
    let version = rest[start..start + end].trim();
    (!version.is_empty()).then(|| version.to_string())
}

fn profile_ctx(
    profile: Profile,
    mtm: MainThreadMarker,
) -> (
    Retained<WKWebsiteDataStore>,
    Retained<WKUserContentController>,
) {
    PROFILES.with(|profiles| {
        let mut profiles = profiles.borrow_mut();
        let ctx = profiles.entry(profile).or_insert_with(|| {
            // SAFETY: main thread (marker held); plain WebKit constructors.
            let store = unsafe {
                match profile {
                    Profile::Normal => WKWebsiteDataStore::defaultDataStore(mtm),
                    Profile::Private(_) => WKWebsiteDataStore::nonPersistentDataStore(mtm),
                }
            };
            let content = unsafe { WKUserContentController::new(mtm) };
            crate::inject::add_handler(&content, mtm);
            content_rules::attach(&content, profile);
            ProfileCtx { store, content }
        });
        (ctx.store.clone(), ctx.content.clone())
    })
}

/// Every profile's content controller (for rule-list swaps).
pub(crate) fn for_each_profile(mut f: impl FnMut(Profile, &WKUserContentController)) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    PROFILES.with(|profiles| {
        for (profile, ctx) in profiles.borrow().iter() {
            f(*profile, &ctx.content);
        }
    });
}

/// Drop a private window's data store and content controller. Its cookies and
/// storage go with it. Call after its tabs are closed.
pub fn drop_private_profile(window_id: WindowId) {
    if mtm().is_none() {
        return;
    }
    PROFILES.with(|profiles| profiles.borrow_mut().remove(&Profile::Private(window_id)));
}

fn new_configuration(profile: Profile, mtm: MainThreadMarker) -> Retained<WKWebViewConfiguration> {
    let (store, content) = profile_ctx(profile, mtm);
    // SAFETY: main thread; configuring a fresh configuration object.
    unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        config.setWebsiteDataStore(&store);
        config.setUserContentController(&content);
        config.setApplicationNameForUserAgent(Some(&ns(&application_name_for_user_agent())));
        let prefs = config.preferences();
        prefs.setElementFullscreenEnabled(true);
        prefs.setFraudulentWebsiteWarningEnabled(true);
        config
    }
}

// ─── Creation ────────────────────────────────────────────────────────────────

/// Create a web view for `profile` as a subview of the host container
/// `parent_view`, sized `width`×`height`, and start loading `url`.
///
/// # Safety
/// `parent_view` must be a valid `NSView` pointer that outlives the browser.
pub unsafe fn create(
    parent_view: *mut c_void,
    width: i32,
    height: i32,
    url: &str,
    profile: Profile,
) -> Option<Browser> {
    let mtm = mtm()?;
    if parent_view.is_null() {
        return None;
    }
    let config = new_configuration(profile, mtm);
    let frame = NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(f64::from(width.max(1)), f64::from(height.max(1))),
    );
    let webview = WKWebView::initWithFrame_configuration(mtm.alloc(), frame, &config);
    // SAFETY: the caller guarantees `parent_view` is a live NSView.
    let parent: &NSView = &*parent_view.cast::<NSView>();
    webview.setAutoresizingMask(
        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    parent.addSubview(&webview);
    let id = register(webview.clone(), profile, None, mtm);
    load(&webview, url);
    Some(Browser(id))
}

/// Wire delegates, KVO and settings onto a new web view and add it to the registry.
fn register(
    webview: Retained<WKWebView>,
    profile: Profile,
    popup_window: Option<Retained<NSWindow>>,
    mtm: MainThreadMarker,
) -> i32 {
    let id = next_id();
    let delegate = WebDelegate::new(mtm, id);
    // SAFETY: main thread; the delegate is retained by the entry for the view's life.
    unsafe {
        webview.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        webview.setUIDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        webview.setAllowsBackForwardNavigationGestures(true);
        webview.setAllowsMagnification(true);
        webview.setInspectable(true);
        for key in OBSERVED_KEYS {
            let _: () = msg_send![&*webview, addObserver: &*delegate, forKeyPath: &*ns(key), options: 1usize, context: std::ptr::null_mut::<c_void>()];
        }
    }
    if let Some(window) = &popup_window {
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    }
    apply_appearance_to(&webview);
    VIEWS.with(|views| {
        views.borrow_mut().insert(
            id,
            Entry {
                webview,
                delegate,
                profile,
                popup_window,
                last: Cell::new(None),
                sync_pending: Cell::new(false),
            },
        )
    });
    id
}

fn load(webview: &WKWebView, url: &str) {
    let url = if url.is_empty() { "about:blank" } else { url };
    let Some(nsurl) = NSURL::URLWithString(&ns(url)) else {
        eprintln!("[engine] not a loadable URL: {url}");
        return;
    };
    // SAFETY: main thread (callers hold the registry, which is main-thread only).
    unsafe {
        webview.loadRequest(&NSURLRequest::requestWithURL(&nsurl));
    }
}

// ─── Handle commands ─────────────────────────────────────────────────────────

impl Browser {
    pub fn identifier(&self) -> i32 {
        self.0
    }

    pub fn is_alive(&self) -> bool {
        with_entry(self.0, |_| ()).is_some()
    }

    pub fn profile(&self) -> Option<Profile> {
        with_entry(self.0, |entry| entry.profile)
    }

    pub fn load_url(&self, url: &str) {
        with_view(self.0, |view| load(view, url));
    }

    pub fn url(&self) -> Option<String> {
        with_view(self.0, |view| {
            unsafe { view.URL() }
                .and_then(|u| u.absoluteString())
                .map(|s| s.to_string())
        })
        .flatten()
    }

    pub fn go_back(&self) {
        with_view(self.0, |view| unsafe { view.goBack() });
    }

    pub fn go_forward(&self) {
        with_view(self.0, |view| unsafe { view.goForward() });
    }

    pub fn reload(&self) {
        with_view(self.0, |view| unsafe { view.reload() });
    }

    pub fn reload_ignore_cache(&self) {
        with_view(self.0, |view| unsafe { view.reloadFromOrigin() });
    }

    pub fn stop(&self) {
        with_view(self.0, |view| unsafe { view.stopLoading() });
    }

    pub fn can_go_back(&self) -> bool {
        with_view(self.0, |view| unsafe { view.canGoBack() }).unwrap_or(false)
    }

    pub fn can_go_forward(&self) -> bool {
        with_view(self.0, |view| unsafe { view.canGoForward() }).unwrap_or(false)
    }

    /// Show or hide the view. The shown tab also takes keyboard focus.
    pub fn set_visible(&self, visible: bool) {
        with_view(self.0, |view| {
            view.setHidden(!visible);
            if visible {
                focus_view(view);
            }
        });
    }

    pub fn focus(&self) {
        with_view(self.0, focus_view);
    }

    pub fn zoom(&self, zoom: Zoom) {
        with_view(self.0, |view| unsafe {
            let current = view.pageZoom();
            let next = match zoom {
                Zoom::Reset => 1.0,
                Zoom::In => ZOOM_LEVELS
                    .iter()
                    .copied()
                    .find(|z| *z > current + 0.001)
                    .unwrap_or(current),
                Zoom::Out => ZOOM_LEVELS
                    .iter()
                    .rev()
                    .copied()
                    .find(|z| *z < current - 0.001)
                    .unwrap_or(current),
            };
            view.setPageZoom(next);
        });
    }

    /// Open Web Inspector for this view (WebKit's DevTools).
    pub fn toggle_inspector(&self) {
        with_view(self.0, |view| unsafe {
            // `_inspector` is WebKit SPI with no public equivalent; Safari's
            // Develop menu can also attach because the view is inspectable.
            let responds: bool = msg_send![view, respondsToSelector: sel!(_inspector)];
            if !responds {
                eprintln!("[engine] Web Inspector SPI unavailable");
                return;
            }
            let inspector: *mut AnyObject = msg_send![view, _inspector];
            if let Some(inspector) = inspector.as_ref() {
                let visible: bool = msg_send![inspector, isVisible];
                if visible {
                    let _: () = msg_send![inspector, close];
                } else {
                    let _: () = msg_send![inspector, show];
                }
            }
        });
    }

    /// Leave element fullscreen and picture-in-picture.
    pub fn exit_fullscreen(&self) {
        with_view(self.0, |view| unsafe {
            view.closeAllMediaPresentationsWithCompletionHandler(None)
        });
    }

    /// Run `script` in the page's own JavaScript world (the button-driven vault fill).
    pub fn evaluate_main_world(&self, script: &str) {
        with_view(self.0, |view| unsafe {
            view.evaluateJavaScript_completionHandler(&ns(script), None)
        });
    }

    /// Find `text` in the page. The first call of a search (`find_next == false`)
    /// selects the first match; later calls step forward or back. Results go to
    /// the host through [`crate::embed::dispatch_find_result`].
    pub fn find(&self, text: &str, forward: bool, match_case: bool, find_next: bool) {
        let id = self.0;
        let Some(mtm) = mtm() else { return };
        let Some(view) = with_view(id, |view| view.retain()) else {
            return;
        };
        if text.is_empty() {
            crate::embed::dispatch_find_result(0, 0, true);
            return;
        }
        let query = serde_json::to_string(text).unwrap_or_default();
        let count_js = format!(
            "(function(q,cs){{var t=document.body?document.body.innerText:'';if(!cs){{t=t.toLowerCase();q=q.toLowerCase();}}var n=0,i=0;while((i=t.indexOf(q,i))!==-1){{n++;i+=q.length;}}return n;}})({query},{match_case})"
        );
        let text = text.to_string();
        let counted = block2::RcBlock::new(
            move |result: *mut AnyObject, _err: *mut objc2_foundation::NSError| {
                let count = unsafe { result.as_ref() }
                    .and_then(|r| r.downcast_ref::<NSNumber>())
                    .map(|n| n.as_i32())
                    .unwrap_or(0);
                let ordinal = FIND.with(|find| {
                    let mut find = find.borrow_mut();
                    let state = find.entry(id).or_insert(FindState {
                        text: String::new(),
                        ordinal: 0,
                    });
                    if !find_next || state.text != text {
                        state.text = text.clone();
                        state.ordinal = if count > 0 { 1 } else { 0 };
                    } else if count > 0 {
                        let step = if forward { 1 } else { -1 };
                        state.ordinal = (state.ordinal - 1 + step).rem_euclid(count) + 1;
                    }
                    state.ordinal
                });
                run_find(id, &text, forward, match_case, count, ordinal);
            },
        );
        // SAFETY: main thread; the isolated client world keeps page JS out of it.
        unsafe {
            view.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
                &ns(&count_js),
                None,
                &WKContentWorld::defaultClientWorld(mtm),
                Some(&counted),
            );
        }
    }

    pub fn stop_finding(&self, clear_selection: bool) {
        FIND.with(|find| find.borrow_mut().remove(&self.0));
        if clear_selection {
            with_view(self.0, |view| unsafe {
                view.evaluateJavaScript_completionHandler(
                    &ns("window.getSelection&&window.getSelection().removeAllRanges()"),
                    None,
                )
            });
        }
    }

    /// Close and release the view. Host-initiated: does not notify the tab
    /// model (the caller already removed or detached the tab).
    pub fn close(&self) {
        close_entry(self.0);
    }
}

fn run_find(id: i32, text: &str, forward: bool, match_case: bool, count: i32, ordinal: i32) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(view) = with_view(id, |view| view.retain()) else {
        return;
    };
    // SAFETY: main thread; fresh find configuration.
    unsafe {
        let config = WKFindConfiguration::new(mtm);
        config.setBackwards(!forward);
        config.setCaseSensitive(match_case);
        config.setWraps(true);
        let done = block2::RcBlock::new(move |result: std::ptr::NonNull<WKFindResult>| {
            let found = result.as_ref().matchFound();
            let (count, ordinal) = if found {
                (count.max(1), ordinal.max(1))
            } else {
                (0, 0)
            };
            crate::embed::dispatch_find_result(count, ordinal, true);
        });
        view.findString_withConfiguration_completionHandler(&ns(text), Some(&config), &done);
    }
}

fn focus_view(view: &WKWebView) {
    if let Some(window) = view.window() {
        window.makeFirstResponder(Some(view));
    }
}

fn close_entry(id: i32) {
    if mtm().is_none() {
        return;
    }
    let Some(entry) = VIEWS.with(|views| views.borrow_mut().remove(&id)) else {
        return;
    };
    FIND.with(|find| find.borrow_mut().remove(&id));
    crate::adblock::forget_tab(id);
    crate::consent::forget_tab(id);
    // SAFETY: main thread; detach every callback before the view goes away.
    unsafe {
        for key in OBSERVED_KEYS {
            let _: () =
                msg_send![&*entry.webview, removeObserver: &*entry.delegate, forKeyPath: &*ns(key)];
        }
        entry.webview.setNavigationDelegate(None);
        entry.webview.setUIDelegate(None);
        entry.webview.stopLoading();
    }
    entry.webview.removeFromSuperview();
    if let Some(window) = &entry.popup_window {
        window.setDelegate(None);
        window.close();
    }
    // Release on a later turn: we may be inside one of this view's own callbacks.
    crate::main_thread::post_local(move || drop(entry));
}

// ─── Engine events → core ───────────────────────────────────────────────────

/// Queue a state sync for `id` (coalesces KVO bursts into one pass).
fn schedule_sync(id: i32) {
    let first = with_entry(id, |entry| !entry.sync_pending.replace(true)).unwrap_or(false);
    if first {
        crate::main_thread::post_local(move || sync_state(id));
    }
}

/// Push changed title / URL / load state to the tab model and history.
fn sync_state(id: i32) {
    let Some((state, previous, private, popup)) = with_entry(id, |entry| {
        entry.sync_pending.set(false);
        let view = &entry.webview;
        // SAFETY: main thread (registry access).
        let state = unsafe {
            PageState {
                url: view
                    .URL()
                    .and_then(|u| u.absoluteString())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                title: view.title().map(|s| s.to_string()).unwrap_or_default(),
                loading: view.isLoading(),
                back: view.canGoBack(),
                forward: view.canGoForward(),
            }
        };
        let previous = entry.last.replace(Some(state.clone())).unwrap_or_default();
        let popup = entry.popup_window.clone();
        (
            state,
            previous,
            matches!(entry.profile, Profile::Private(_)),
            popup,
        )
    }) else {
        return;
    };
    if let Some(window) = popup {
        let title = if state.title.is_empty() {
            state.url.clone()
        } else {
            state.title.clone()
        };
        window.setTitle(&ns(&title));
        return;
    }
    if state.url != previous.url && !state.url.is_empty() {
        if !private {
            crate::history::record_visit(&state.url);
        }
        if !crate::private_window::note_url(id, &state.url) {
            crate::tabs::note_url(id, &state.url);
        }
    }
    if state.title != previous.title || (state.url != previous.url && !state.title.is_empty()) {
        if !private && !state.url.is_empty() {
            crate::history::update_title(&state.url, &state.title);
        }
        if !crate::private_window::note_title(id, &state.title) {
            crate::tabs::note_title(id, &state.title);
        }
    }
    if (state.loading, state.back, state.forward)
        != (previous.loading, previous.back, previous.forward)
        && !crate::private_window::note_load_state(id, state.loading, state.back, state.forward)
    {
        crate::tabs::note_load_state(id, state.loading, state.back, state.forward);
    }
}

/// A page closed itself (`window.close()`): drop its tab from the model.
fn page_closed(id: i32) {
    let popup = with_entry(id, |entry| entry.popup_window.is_some()).unwrap_or(false);
    if popup {
        close_entry(id);
        return;
    }
    crate::main_thread::post(move || {
        if crate::private_window::is_private_cef(id) {
            crate::private_window::note_closed(id);
        } else {
            crate::tabs::note_closed(id);
        }
    });
}

/// A link asked for a new tab (`target=_blank`, ⌘-click, window.open without size).
fn open_in_new_tab(source: i32, url: String, foreground: bool) {
    crate::main_thread::post(move || {
        if crate::private_window::is_private_cef(source) {
            crate::private_window::request_open_tab(source, &url, foreground);
        } else {
            crate::tabs::request_open_tab(&url, foreground);
        }
    });
}

/// Create a popup window for a script-opened, sized `window.open` (OAuth and
/// payment flows need `window.opener` and `window.close()`).
fn create_popup(
    source: i32,
    config: &WKWebViewConfiguration,
    width: f64,
    height: f64,
) -> Option<Retained<WKWebView>> {
    let mtm = MainThreadMarker::new()?;
    let profile = with_entry(source, |entry| entry.profile)?;
    let frame = NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(width.max(320.0), height.max(240.0)),
    );
    let style = objc2_app_kit::NSWindowStyleMask::Titled
        | objc2_app_kit::NSWindowStyleMask::Closable
        | objc2_app_kit::NSWindowStyleMask::Miniaturizable
        | objc2_app_kit::NSWindowStyleMask::Resizable;
    // SAFETY: main thread; standard window + web view construction.
    unsafe {
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            frame,
            style,
            objc2_app_kit::NSBackingStoreType::Buffered,
            false,
        );
        window.setReleasedWhenClosed(false);
        let webview = WKWebView::initWithFrame_configuration(mtm.alloc(), frame, config);
        window.setContentView(Some(&webview));
        window.center();
        register(webview.clone(), profile, Some(window.clone()), mtm);
        window.makeKeyAndOrderFront(None);
        Some(webview)
    }
}

// ─── Appearance ──────────────────────────────────────────────────────────────

/// Force light (`Some(false)`), dark (`Some(true)`), or follow the window (`None`)
/// for every web view, now and for views created later.
pub fn set_dark(dark: Option<bool>) {
    if mtm().is_none() {
        return;
    }
    DARK.with(|d| d.set(dark));
    VIEWS.with(|views| {
        for entry in views.borrow().values() {
            apply_appearance_to(&entry.webview);
        }
    });
}

fn apply_appearance_to(view: &WKWebView) {
    let dark = DARK.with(Cell::get);
    // SAFETY: main thread; named system appearances always exist.
    let appearance = dark.and_then(|dark| unsafe {
        let name = if dark {
            objc2_app_kit::NSAppearanceNameDarkAqua
        } else {
            objc2_app_kit::NSAppearanceNameAqua
        };
        NSAppearance::appearanceNamed(name)
    });
    view.setAppearance(appearance.as_deref());
}

/// Ids of every live web view (tabs and popups).
pub fn live_ids() -> Vec<i32> {
    if MainThreadMarker::new().is_none() {
        return Vec::new();
    }
    VIEWS.with(|views| views.borrow().keys().copied().collect())
}

/// Close every web view (app quit).
pub fn close_all() {
    for id in live_ids() {
        close_entry(id);
    }
}

pub(crate) fn browser_from_view(view: &WKWebView) -> Option<i32> {
    MainThreadMarker::new()?;
    VIEWS.with(|views| {
        views
            .borrow()
            .iter()
            .find(|(_, entry)| std::ptr::eq(&*entry.webview, view))
            .map(|(id, _)| *id)
    })
}
