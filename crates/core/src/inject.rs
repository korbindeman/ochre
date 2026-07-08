//! Content-script injection substrate — document-start, isolated world (P2-14, ADR-0017).
//!
//! This is the *one hard thing built once* that autofill (P2-15/16), cosmetic
//! ad-block (P2-21), cookie-consent (P2-22), and userscripts / Tweaks (spec §20)
//! all ride. It gives the core two capabilities against any page:
//!
//! 1. **Run a content script at document-start in an isolated world** — a
//!    JavaScript execution context that shares the page's DOM but *not* its
//!    global scope, so page JS can't reach (or forge) our bridge, and our script
//!    runs before the page's own scripts.
//! 2. **A bidirectional page↔core message channel** — the script asks the core
//!    for data and the core answers; the credential/rule never lives in the
//!    injected source (the fix for the autofill v0's main-world, password-baked-in
//!    approach).
//!
//! ## How it works (all internal — no external CDP port)
//!
//! CEF exposes the Chrome DevTools Protocol *in-process* via
//! [`ImplBrowserHost::send_dev_tools_message`] + an observer registered with
//! [`ImplBrowserHost::add_dev_tools_message_observer`]. Per browser we:
//!
//! - `Page.enable` + `Runtime.enable` — turn on the two domains we drive.
//! - `Runtime.addBinding{name, executionContextName}` — expose a function
//!   `window.__browser_core_send(json)` **only in the isolated world**; calling it
//!   fires a `Runtime.bindingCalled` event back to us (the page→core edge).
//! - `Page.addScriptToEvaluateOnNewDocument{source, worldName, runImmediately}` —
//!   register the bootstrap + each consumer's script to run at document-start in
//!   that same named world, for every future navigation.
//!
//! When the page calls the binding, `Runtime.bindingCalled` carries the payload
//! **and the originating `executionContextId`** — so the core→page reply is a
//! `Runtime.evaluate{expression, contextId}` targeting that exact isolated
//! context. Request/response needs no separate context tracking; the incoming
//! event tells us where to answer.
//!
//! ## Frame coverage
//!
//! `Page.addScriptToEvaluateOnNewDocument` + `Runtime.addBinding` apply per-frame,
//! so the substrate reaches the **main frame and every same-origin (same-process)
//! iframe** automatically (verified). **Cross-origin iframes (OOPIFs)** are not
//! covered — see [ADR-0027]: CEF 149's DevTools observer delivers *results* but
//! not *events* from auto-attached child sessions, so their `bindingCalled` never
//! arrives; the real fix is a render-process-handler path (P2-24).
//!
//! ## Registry
//!
//! Consumers [`register`] a `{channel, script, handler}` at startup (before any
//! browser exists). The substrate installs itself on a browser only if at least
//! one consumer is registered — so a release build in which nothing yet uses the
//! substrate pays zero per-page cost.
//!
//! Everything here runs on the CEF UI thread (the app's main thread in embedded
//! mode): `install`/`uninstall` from the life-span handler, the observer
//! callbacks from CEF. The consumer registry is process-global (`fn`-pointer
//! handlers are `Send + Sync`); the per-browser observer registrations are held
//! in a thread-local, since they are UI-thread-only and need no `Send`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use cef::*;

/// Name of the isolated world our scripts and binding live in. Any name works;
/// it only has to be consistent between `addBinding` and `addScriptToEvaluateOnNewDocument`.
const ISOLATED_WORLD: &str = "BrowserCoreWorld";

/// The CDP binding the isolated world calls to reach the core. Deliberately
/// obscure; it exists only in the isolated world, never the page's main world.
const SEND_BINDING: &str = "__browser_core_send";

// ─── Public consumer API ─────────────────────────────────────────────────

/// A message from an isolated-world content script, routed to the consumer that
/// owns its `channel`.
pub struct Message<'a> {
    /// The consumer channel the script addressed.
    pub channel: &'a str,
    /// The `data` payload the script sent (arbitrary JSON; `Null` if omitted).
    pub data: &'a serde_json::Value,
    /// The CEF browser identifier of the tab the message came from — for
    /// consumers that key per-tab state (e.g. consent's rejected-banner count).
    pub tab_id: i32,
}

/// A consumer's message handler. Runs on the UI thread. Returns `Some(reply)` to
/// answer the script (delivered to its pending `send(...)` promise), or `None`
/// for fire-and-forget.
pub type Handler = fn(&Message) -> Option<serde_json::Value>;

/// A registered rider on the substrate: a document-start `script` (isolated
/// world) plus the `handler` for the messages it sends on `channel`.
///
/// The script is `&'static str` on purpose: the injected source is *generic*
/// (form detection, element matching, …) and must never carry per-page secrets —
/// those cross only via the message channel, on demand (ADR-0017).
pub struct Consumer {
    pub channel: &'static str,
    pub script: &'static str,
    pub handler: Handler,
}

/// Register a consumer. Call at startup, before any browser is created.
pub fn register(consumer: Consumer) {
    consumers().push(consumer);
}

// ─── Registry & per-browser state ────────────────────────────────────────

static CONSUMERS: Mutex<Vec<Consumer>> = Mutex::new(Vec::new());
static NEXT_MSG_ID: AtomicI32 = AtomicI32::new(1);

/// Tabs whose first real document may have raced `addScriptToEvaluateOnNewDocument`
/// registration — [`bootstrap_on_load_end`] runs the scripts explicitly once.
static NEEDS_FIRST_BOOTSTRAP: OnceLock<Mutex<HashMap<i32, bool>>> = OnceLock::new();

/// In-flight CDP calls for the load-end bootstrap state machine (UI thread only).
static PENDING_CDP: OnceLock<Mutex<HashMap<i32, PendingCdp>>> = OnceLock::new();

/// First navigations deferred until the injection substrate is registered — see
/// [`defer_first_navigation`] / [`dispatch_pending_navigation`].
static PENDING_NAV: OnceLock<Mutex<HashMap<i32, String>>> = OnceLock::new();

/// Staged URL for the next browser whose id is not known until `on_after_created`
/// (async `browser_host_create_browser`).
static STAGED_FIRST_NAV: Mutex<Option<String>> = Mutex::new(None);

fn pending_nav() -> &'static Mutex<HashMap<i32, String>> {
    PENDING_NAV.get_or_init(|| Mutex::new(HashMap::new()))
}

/// URL used for the initial hop while CDP scripts are wired in [`install_for_browser`].
pub const BLANK_URL: &str = "about:blank";

/// Stage the real first URL for the next browser created via async CEF create.
pub fn stage_first_navigation(url: &str) {
    if url == BLANK_URL {
        return;
    }
    *STAGED_FIRST_NAV.lock().unwrap_or_else(|p| p.into_inner()) = Some(url.to_string());
}

/// Attach a staged first URL to `browser_id` (call from `on_after_created`).
pub fn claim_staged_navigation(browser_id: i32) {
    let url = STAGED_FIRST_NAV
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    if let Some(url) = url {
        defer_first_navigation(browser_id, &url);
    }
}

/// Queue the real first URL for `browser_id`. The browser must be created on
/// [`BLANK_URL`]; [`dispatch_pending_navigation`] loads it from `on_load_end`.
pub fn defer_first_navigation(browser_id: i32, url: &str) {
    if url == BLANK_URL {
        return;
    }
    pending_nav()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(browser_id, url.to_string());
}

/// If a deferred first navigation is queued for `browser_id`, return it (once).
pub fn take_pending_navigation(browser_id: i32) -> Option<String> {
    pending_nav()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&browser_id)
}

/// Load a deferred first navigation on the main frame, if any. Call from
/// `on_load_end` after the blank hop completes. Navigation is posted with a
/// short delay so in-process CDP script registration can finish first.
pub fn dispatch_pending_navigation(browser: &mut Browser, frame: &mut Frame) {
    if frame.is_main() == 0 {
        return;
    }
    let id = browser.identifier();
    let Some(url) = take_pending_navigation(id) else {
        return;
    };
    eprintln!("[inject] scheduling deferred navigation tab={id} → {url}");
    let mut task = DeferredNavTask::new(id, url);
    post_delayed_task(ThreadId::UI, Some(&mut task), 250);
}

/// Explicitly run bootstrap + consumer scripts in the isolated world after the
/// main frame finishes loading. Covers the first-load race where
/// `Page.addScriptToEvaluateOnNewDocument` registration in `on_after_created`
/// completes after the document has already started (P2-14 timing note).
pub fn bootstrap_on_load_end(browser: &mut Browser, frame: &mut Frame) {
    if frame.is_main() == 0 {
        return;
    }
    let id = browser.identifier();
    if !needs_first_bootstrap(id) {
        return;
    }
    if consumers().is_empty() {
        return;
    }
    let url = CefString::from(&frame.url()).to_string();
    if url == BLANK_URL || url.is_empty() {
        return;
    }
    let Some(host) = browser.host() else {
        return;
    };
    let scripts = bootstrap_scripts();
    let msg_id = next_id();
    pending_cdp()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(
            msg_id,
            PendingCdp::FrameTree {
                browser_id: id,
                scripts,
            },
        );
    send(&host, msg_get_frame_tree(msg_id));
    eprintln!("[inject] load-end bootstrap tab={id} url={url}");
}

wrap_task! {
    struct DeferredNavTask {
        browser_id: i32,
        url: String,
    }

    impl Task {
        fn execute(&self) {
            let Some(handler) = crate::client::BrowserHandler::instance() else {
                return;
            };
            let mut guard = match handler.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            let mut browsers = guard.matching_browsers(|id| id == self.browser_id);
            let Some(browser) = browsers.first_mut() else {
                return;
            };
            let Some(mut frame) = browser.main_frame() else {
                return;
            };
            eprintln!(
                "[inject] deferred navigation tab={} → {}",
                self.browser_id, self.url
            );
            let cef_url = CefString::from(self.url.as_str());
            frame.load_url(Some(&cef_url));
        }
    }
}

thread_local! {
    /// The observer registration per browser id. Holding the [`Registration`]
    /// keeps the subscription alive; dropping it (on browser close) tears it
    /// down. UI-thread only.
    static REGISTRATIONS: RefCell<HashMap<i32, Registration>> = RefCell::new(HashMap::new());
}

fn consumers() -> std::sync::MutexGuard<'static, Vec<Consumer>> {
    CONSUMERS.lock().unwrap_or_else(|p| p.into_inner())
}

fn next_id() -> i32 {
    NEXT_MSG_ID.fetch_add(1, Ordering::Relaxed)
}

fn needs_first_bootstrap_map() -> &'static Mutex<HashMap<i32, bool>> {
    NEEDS_FIRST_BOOTSTRAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pending_cdp() -> &'static Mutex<HashMap<i32, PendingCdp>> {
    PENDING_CDP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn needs_first_bootstrap(browser_id: i32) -> bool {
    needs_first_bootstrap_map()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&browser_id)
        .copied()
        .unwrap_or(false)
}

fn mark_needs_bootstrap(browser_id: i32) {
    needs_first_bootstrap_map()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(browser_id, true);
}

fn clear_needs_bootstrap(browser_id: i32) {
    needs_first_bootstrap_map()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(browser_id, false);
}

fn clear_bootstrap_state(browser_id: i32) {
    needs_first_bootstrap_map()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&browser_id);
    let mut pending = pending_cdp().lock().unwrap_or_else(|p| p.into_inner());
    pending.retain(|_, op| op.browser_id() != browser_id);
}

fn bootstrap_scripts() -> Vec<&'static str> {
    let mut scripts = vec![BOOTSTRAP_JS];
    scripts.extend(consumers().iter().map(|c| c.script));
    scripts
}

enum PendingCdp {
    FrameTree {
        browser_id: i32,
        scripts: Vec<&'static str>,
    },
    CreateWorld {
        browser_id: i32,
        scripts: Vec<&'static str>,
    },
    Evaluate {
        browser_id: i32,
        context_id: i64,
        scripts: Vec<&'static str>,
        index: usize,
    },
}

impl PendingCdp {
    fn browser_id(&self) -> i32 {
        match self {
            Self::FrameTree { browser_id, .. }
            | Self::CreateWorld { browser_id, .. }
            | Self::Evaluate { browser_id, .. } => *browser_id,
        }
    }
}

// ─── Install / uninstall (life-span handler) ─────────────────────────────

/// Attach the substrate to a freshly created browser: register the DevTools
/// observer, enable the domains, expose the binding, and register the bootstrap
/// + every consumer script to run at document-start in the isolated world.
///
/// A no-op when no consumer is registered — the substrate costs nothing until
/// something rides it. UI thread only (called from `on_after_created`).
pub fn install_for_browser(browser: &mut Browser) {
    let scripts: Vec<&'static str> = {
        let consumers = consumers();
        if consumers.is_empty() {
            return;
        }
        consumers.iter().map(|c| c.script).collect()
    };

    let id = browser.identifier();
    let Some(host) = browser.host() else {
        eprintln!("[inject] no host for tab={id}");
        return;
    };

    let mut observer = InjectObserver::new();
    let Some(registration) = host.add_dev_tools_message_observer(Some(&mut observer)) else {
        eprintln!("[inject] add_dev_tools_message_observer failed tab={id}");
        return;
    };
    REGISTRATIONS.with(|r| r.borrow_mut().insert(id, registration));

    // The first message attaches the DevTools agent; the rest follow on the same
    // session. Order matters: the binding must exist before the isolated world is
    // created on the next navigation, or the bootstrap's send would find it missing.
    // These apply to the main frame and every same-origin iframe automatically;
    // cross-origin iframes are a separate DevTools target CEF can't relay events
    // from (ADR-0027).
    send(&host, msg_domain_enable(next_id(), "Page"));
    send(&host, msg_domain_enable(next_id(), "Runtime"));
    send(&host, msg_add_binding(next_id()));
    send(&host, msg_add_script(next_id(), BOOTSTRAP_JS));
    for script in &scripts {
        send(&host, msg_add_script(next_id(), script));
    }
    mark_needs_bootstrap(id);
    eprintln!(
        "[inject] substrate installed tab={id} ({} consumer(s))",
        scripts.len()
    );
}

/// Drop the substrate for a closing browser (its observer registration).
/// UI thread only (called from `on_before_close`).
pub fn uninstall_for_browser(id: i32) {
    REGISTRATIONS.with(|r| r.borrow_mut().remove(&id));
    clear_bootstrap_state(id);
}

/// Evaluate `expression` in the **main world of a browser's main frame** — the
/// *core-initiated* injection path, distinct from the reply-only page→core
/// substrate above (which only ever answers a `bindingCalled`).
///
/// Used by button-driven vault autofill (ADR-0029): the fill snippet is
/// self-contained and the credential it carries is exposed to the page anyway
/// (that is the whole point of a fill), so it runs in the **main world** — which
/// means no isolated-world `executionContextId` to track, and a `Runtime.evaluate`
/// with no `contextId` targets the browser's default context (the main frame's
/// main world). Reaches only the **main frame**, not sub-frames. UI thread only.
pub fn evaluate_main_world(host: &BrowserHost, expression: &str) {
    send(host, msg_evaluate_default(next_id(), expression));
}

fn send(host: &BrowserHost, message: Vec<u8>) {
    if host.send_dev_tools_message(Some(&message)) == 0 {
        eprintln!("[inject] send_dev_tools_message failed");
    }
}

// ─── DevTools message observer ───────────────────────────────────────────

// Receives CDP events + method results for the browser it is registered on.
// `Runtime.bindingCalled` is the page→core edge; `on_dev_tools_method_result`
// drives the load-end bootstrap state machine.
wrap_dev_tools_message_observer! {
    struct InjectObserver;

    impl DevToolsMessageObserver {
        fn on_dev_tools_event(
            &self,
            browser: Option<&mut Browser>,
            method: Option<&CefString>,
            params: Option<&[u8]>,
        ) {
            let (Some(browser), Some(method), Some(params)) = (browser, method, params) else {
                return;
            };
            if method.to_string() != "Runtime.bindingCalled" {
                return;
            }
            handle_binding_called(browser, params);
        }

        fn on_dev_tools_method_result(
            &self,
            browser: Option<&mut Browser>,
            message_id: i32,
            success: i32,
            result: Option<&[u8]>,
        ) {
            handle_method_result(browser, message_id, success, result);
        }
    }
}

/// Handle a `Runtime.bindingCalled` event: parse it, route the payload to the
/// consumer that owns its channel, and — if the consumer replied — evaluate the
/// reply back into the *same* isolated-world context that sent it.
fn handle_binding_called(browser: &mut Browser, params: &[u8]) {
    let Some(call) = parse_binding_called(params) else {
        return;
    };
    let Some(incoming) = parse_payload(&call.payload) else {
        return;
    };
    let Some(reply) = route(&incoming, browser.identifier()) else {
        return; // fire-and-forget, or no consumer for this channel
    };
    let envelope = serde_json::json!({ "id": incoming.id, "data": reply }).to_string();
    let expr = receive_expression(&envelope);
    if let Some(host) = browser.host() {
        send(
            &host,
            msg_evaluate(next_id(), call.execution_context_id, &expr),
        );
    }
}

fn route(incoming: &Incoming, tab_id: i32) -> Option<serde_json::Value> {
    dispatch(&consumers(), incoming, tab_id)
}

fn handle_method_result(
    browser: Option<&mut Browser>,
    message_id: i32,
    success: i32,
    result: Option<&[u8]>,
) {
    let pending = pending_cdp()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&message_id);
    let Some(pending) = pending else {
        return;
    };
    if success == 0 {
        eprintln!("[inject] CDP method failed id={message_id}");
        return;
    }
    let Some(result) = result else {
        eprintln!("[inject] CDP method {message_id} returned no result");
        return;
    };
    let Some(browser) = browser else {
        return;
    };
    let Some(host) = browser.host() else {
        return;
    };

    match pending {
        PendingCdp::FrameTree {
            browser_id,
            scripts,
        } => {
            let Some(frame_id) = parse_main_frame_id(result) else {
                eprintln!("[inject] bootstrap: no main frame id tab={browser_id}");
                return;
            };
            let msg_id = next_id();
            pending_cdp()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(
                    msg_id,
                    PendingCdp::CreateWorld {
                        browser_id,
                        scripts,
                    },
                );
            send(&host, msg_create_isolated_world(msg_id, &frame_id));
        }
        PendingCdp::CreateWorld {
            browser_id,
            scripts,
        } => {
            let Some(context_id) = parse_execution_context_id(result) else {
                eprintln!("[inject] bootstrap: no executionContextId tab={browser_id}");
                return;
            };
            if scripts.is_empty() {
                clear_needs_bootstrap(browser_id);
                return;
            }
            let msg_id = next_id();
            let script = scripts[0];
            pending_cdp()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(
                    msg_id,
                    PendingCdp::Evaluate {
                        browser_id,
                        context_id,
                        scripts,
                        index: 0,
                    },
                );
            send(&host, msg_evaluate_bootstrap(msg_id, context_id, script));
        }
        PendingCdp::Evaluate {
            browser_id,
            context_id,
            scripts,
            index,
        } => {
            let next = index + 1;
            if next >= scripts.len() {
                clear_needs_bootstrap(browser_id);
                eprintln!("[inject] load-end bootstrap complete tab={browser_id}");
                return;
            }
            let msg_id = next_id();
            let script = scripts[next];
            pending_cdp()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(
                    msg_id,
                    PendingCdp::Evaluate {
                        browser_id,
                        context_id,
                        scripts,
                        index: next,
                    },
                );
            send(&host, msg_evaluate_bootstrap(msg_id, context_id, script));
        }
    }
}

/// Pure dispatch: find the consumer for `incoming.channel` and run its handler.
/// Split from [`route`] so it is testable without touching the global registry.
fn dispatch(consumers: &[Consumer], incoming: &Incoming, tab_id: i32) -> Option<serde_json::Value> {
    let consumer = consumers.iter().find(|c| c.channel == incoming.channel)?;
    (consumer.handler)(&Message {
        channel: &incoming.channel,
        data: &incoming.data,
        tab_id,
    })
}

// ─── Wire parsing / message building (pure) ──────────────────────────────

/// The relevant fields of a `Runtime.bindingCalled` event for our binding.
struct BindingCall {
    payload: String,
    execution_context_id: i64,
}

/// A parsed page→core message: `{channel, id, data}`.
struct Incoming {
    channel: String,
    id: i64,
    data: serde_json::Value,
}

/// Parse a `Runtime.bindingCalled` params blob, keeping only calls to our
/// binding. `None` for any other binding or a malformed event.
fn parse_binding_called(params: &[u8]) -> Option<BindingCall> {
    let v: serde_json::Value = serde_json::from_slice(params).ok()?;
    if v.get("name")?.as_str()? != SEND_BINDING {
        return None;
    }
    Some(BindingCall {
        payload: v.get("payload")?.as_str()?.to_string(),
        execution_context_id: v.get("executionContextId")?.as_i64()?,
    })
}

/// Parse the JSON payload the bootstrap sends: `{channel, id, data}`.
fn parse_payload(payload: &str) -> Option<Incoming> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    Some(Incoming {
        channel: v.get("channel")?.as_str()?.to_string(),
        id: v.get("id")?.as_i64()?,
        data: v.get("data").cloned().unwrap_or(serde_json::Value::Null),
    })
}

/// Build the `window.__browserCore.__receive(...)` call that delivers a reply
/// envelope into the isolated world. The envelope JSON is embedded as a JS string
/// literal (JSON is valid JS) and re-parsed inside `__receive`.
fn receive_expression(envelope_json: &str) -> String {
    let literal = serde_json::to_string(envelope_json).unwrap_or_else(|_| "\"\"".to_string());
    format!("window.__browserCore&&window.__browserCore.__receive({literal});")
}

fn msg_domain_enable(id: i32, domain: &str) -> Vec<u8> {
    to_vec(&serde_json::json!({ "id": id, "method": format!("{domain}.enable") }))
}

fn msg_add_binding(id: i32) -> Vec<u8> {
    to_vec(&serde_json::json!({
        "id": id,
        "method": "Runtime.addBinding",
        "params": { "name": SEND_BINDING, "executionContextName": ISOLATED_WORLD },
    }))
}

fn msg_add_script(id: i32, source: &str) -> Vec<u8> {
    to_vec(&serde_json::json!({
        "id": id,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "params": { "source": source, "worldName": ISOLATED_WORLD, "runImmediately": true },
    }))
}

fn msg_get_frame_tree(id: i32) -> Vec<u8> {
    to_vec(&serde_json::json!({ "id": id, "method": "Page.getFrameTree" }))
}

fn msg_create_isolated_world(id: i32, frame_id: &str) -> Vec<u8> {
    to_vec(&serde_json::json!({
        "id": id,
        "method": "Page.createIsolatedWorld",
        "params": {
            "frameId": frame_id,
            "worldName": ISOLATED_WORLD,
            "grantUniveralAccess": false,
        },
    }))
}

fn parse_main_frame_id(result: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(result).ok()?;
    let id = v.get("frameTree")?.get("frame")?.get("id")?;
    match id {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn parse_execution_context_id(result: &[u8]) -> Option<i64> {
    let v: serde_json::Value = serde_json::from_slice(result).ok()?;
    v.get("executionContextId")?.as_i64()
}

fn msg_evaluate(id: i32, context_id: i64, expression: &str) -> Vec<u8> {
    to_vec(&serde_json::json!({
        "id": id,
        "method": "Runtime.evaluate",
        "params": { "expression": expression, "contextId": context_id },
    }))
}

/// Wrap a static script for explicit load-end injection. A freshly created
/// isolated context can evaluate before `document.documentElement` exists.
fn wrap_load_end_script(source: &str) -> String {
    let literal = serde_json::to_string(source).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        "(function(){{var s={literal};function run(){{if(!document.documentElement){{requestAnimationFrame(run);return;}}(0,eval)(s);}}run();}})();"
    )
}

fn msg_evaluate_bootstrap(id: i32, context_id: i64, source: &str) -> Vec<u8> {
    msg_evaluate(id, context_id, &wrap_load_end_script(source))
}

/// A `Runtime.evaluate` with **no `contextId`** — the browser's default context
/// (the main frame's main world). The core-initiated counterpart to
/// [`msg_evaluate`], which targets a specific isolated-world context for a reply.
fn msg_evaluate_default(id: i32, expression: &str) -> Vec<u8> {
    to_vec(&serde_json::json!({
        "id": id,
        "method": "Runtime.evaluate",
        "params": { "expression": expression },
    }))
}

/// Serialize a CDP message we built ourselves. The value is always a literal
/// object of known-serializable parts, so failure is impossible in practice.
fn to_vec(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("CDP message serializes")
}

// ─── The bootstrap script (isolated world, document-start) ───────────────

/// Runs first in the isolated world on every document. Installs `window.__browserCore`:
/// - `send(channel, data)` → a Promise resolved with the core's reply.
/// - `__receive(json)` → called by the core (via `Runtime.evaluate`) to resolve it.
///
/// Idempotent (guards on re-run). Lives only in the isolated world, so the page
/// can neither see nor forge it.
const BOOTSTRAP_JS: &str = r#"(function () {
  if (window.__browserCore) return;
  var pending = Object.create(null);
  var seq = 0;
  window.__browserCore = {
    send: function (channel, data) {
      var id = ++seq;
      return new Promise(function (resolve) {
        pending[id] = resolve;
        try {
          window.__browser_core_send(JSON.stringify({ channel: channel, id: id, data: data }));
        } catch (e) {
          delete pending[id];
          resolve(undefined);
        }
      });
    },
    __receive: function (json) {
      var m;
      try { m = JSON.parse(json); } catch (e) { return; }
      var resolve = pending[m.id];
      if (resolve) { delete pending[m.id]; resolve(m.data); }
    }
  };
})();"#;

// ─── Debug-only demo consumer (live round-trip proof) ────────────────────

/// Register a demo consumer that exercises the full round-trip against a real
/// page, so the substrate is verifiable end-to-end without a production rider
/// (autofill migrates onto the substrate in P2-15). Debug builds only, gated on
/// `BROWSER_INJECT_DEMO` — see [`crate::embed::initialize`].
#[cfg(debug_assertions)]
pub fn register_demo_consumer() {
    register(Consumer {
        channel: "inject-demo",
        script: DEMO_SCRIPT,
        handler: demo_handler,
    });
}

/// Echoes the page's `ping` back as `pong`, prefixed so the reply is provably
/// core-authored (not something the page could have written itself).
#[cfg(debug_assertions)]
fn demo_handler(msg: &Message) -> Option<serde_json::Value> {
    let ping = msg.data.get("ping").and_then(|v| v.as_str()).unwrap_or("");
    Some(serde_json::json!({ "pong": format!("core-saw:{ping}") }))
}

/// The demo content script (isolated world). On load it sends a ping to the core
/// and, on the reply, writes the result to a **DOM attribute** — the DOM is shared
/// with the page's main world, so an external CDP client (which evaluates in the
/// main world) can read the marker to confirm the round-trip, while
/// `window.__browserCore` stays invisible there, proving isolation.
#[cfg(debug_assertions)]
const DEMO_SCRIPT: &str = r#"(function () {
  if (!window.__browserCore) return;
  window.__browserCore.send('inject-demo', { host: location.host, ping: 'from-isolated' })
    .then(function (reply) {
      var mark = (reply && reply.pong) ? ('ok:' + reply.pong) : 'no-reply';
      document.documentElement.setAttribute('data-browser-inject-demo', mark);
    });
})();"#;

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn params(name: &str, payload: &str, ctx: i64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "name": name,
            "payload": payload,
            "executionContextId": ctx,
        }))
        .unwrap()
    }

    #[test]
    fn parse_binding_called_keeps_only_our_binding() {
        let ours = params(SEND_BINDING, "{}", 7);
        let call = parse_binding_called(&ours).expect("our binding parses");
        assert_eq!(call.execution_context_id, 7);
        assert_eq!(call.payload, "{}");

        // A different binding (e.g. some page's own) is ignored.
        assert!(parse_binding_called(&params("someOtherBinding", "{}", 1)).is_none());
        // Malformed bytes are ignored.
        assert!(parse_binding_called(b"not json").is_none());
    }

    #[test]
    fn parse_payload_reads_channel_id_data() {
        let incoming =
            parse_payload(r#"{"channel":"autofill","id":3,"data":{"host":"example.com"}}"#)
                .expect("valid payload");
        assert_eq!(incoming.channel, "autofill");
        assert_eq!(incoming.id, 3);
        assert_eq!(incoming.data["host"], "example.com");

        // Missing data defaults to Null (a fire-and-forget signal is still valid).
        let no_data = parse_payload(r#"{"channel":"c","id":1}"#).expect("data optional");
        assert!(no_data.data.is_null());

        // Missing required fields ⇒ None.
        assert!(parse_payload(r#"{"channel":"c"}"#).is_none());
        assert!(parse_payload("garbage").is_none());
    }

    #[test]
    fn dispatch_routes_by_channel_and_runs_handler() {
        fn echo(msg: &Message) -> Option<serde_json::Value> {
            Some(serde_json::json!({ "saw": msg.data.clone() }))
        }
        let consumers = vec![Consumer {
            channel: "echo",
            script: "",
            handler: echo,
        }];

        let hit = Incoming {
            channel: "echo".into(),
            id: 1,
            data: serde_json::json!({ "x": 1 }),
        };
        assert_eq!(
            dispatch(&consumers, &hit, 7),
            Some(serde_json::json!({ "saw": { "x": 1 } }))
        );

        // Unknown channel ⇒ no reply.
        let miss = Incoming {
            channel: "nope".into(),
            id: 2,
            data: serde_json::Value::Null,
        };
        assert!(dispatch(&consumers, &miss, 7).is_none());
    }

    #[test]
    fn receive_expression_embeds_reply_as_safe_js_literal() {
        // A reply containing quotes / backslashes must survive as a JS string.
        let envelope = r#"{"id":1,"data":{"pong":"a\"b"}}"#;
        let expr = receive_expression(envelope);
        assert!(expr.starts_with("window.__browserCore&&window.__browserCore.__receive("));
        assert!(expr.ends_with(");"));
        // The inner content is a single JS string literal (double-encoded JSON):
        // no raw unescaped inner quote breaks out of it.
        let inner = expr
            .trim_start_matches("window.__browserCore&&window.__browserCore.__receive(")
            .trim_end_matches(");");
        let decoded: String = serde_json::from_str(inner).expect("inner is a JSON string literal");
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn add_script_targets_the_isolated_world_at_document_start() {
        let msg = msg_add_script(5, "console.log(1)");
        let v: serde_json::Value = serde_json::from_slice(&msg).unwrap();
        assert_eq!(v["method"], "Page.addScriptToEvaluateOnNewDocument");
        assert_eq!(v["params"]["worldName"], ISOLATED_WORLD);
        assert_eq!(v["params"]["runImmediately"], true);
        assert_eq!(v["params"]["source"], "console.log(1)");
        assert_eq!(v["id"], 5);
    }

    #[test]
    fn add_binding_binds_only_the_isolated_world() {
        let v: serde_json::Value = serde_json::from_slice(&msg_add_binding(9)).unwrap();
        assert_eq!(v["method"], "Runtime.addBinding");
        assert_eq!(v["params"]["name"], SEND_BINDING);
        assert_eq!(v["params"]["executionContextName"], ISOLATED_WORLD);
    }

    #[test]
    fn evaluate_targets_the_originating_context() {
        let v: serde_json::Value = serde_json::from_slice(&msg_evaluate(2, 42, "1+1")).unwrap();
        assert_eq!(v["method"], "Runtime.evaluate");
        assert_eq!(v["params"]["expression"], "1+1");
        assert_eq!(v["params"]["contextId"], 42);
    }

    #[test]
    fn evaluate_default_omits_context_for_the_main_world() {
        // The core-initiated fill path targets the main frame's default context —
        // no contextId, so CEF runs it in the main world (ADR-0029).
        let v: serde_json::Value = serde_json::from_slice(&msg_evaluate_default(3, "1+1")).unwrap();
        assert_eq!(v["method"], "Runtime.evaluate");
        assert_eq!(v["params"]["expression"], "1+1");
        assert!(
            v["params"].get("contextId").is_none(),
            "main-world evaluate must not pin a contextId"
        );
    }

    #[test]
    fn wrap_load_end_script_defers_until_document_element_exists() {
        let wrapped = wrap_load_end_script("document.title='x'");
        assert!(wrapped.contains("requestAnimationFrame"));
        assert!(wrapped.contains("document.documentElement"));
        assert!(wrapped.contains("document.title='x'"));
    }

    #[test]
    fn parse_main_frame_id_accepts_string_and_number_ids() {
        let string_id = serde_json::json!({
            "frameTree": { "frame": { "id": "ABC123", "url": "https://example.com/" } }
        });
        assert_eq!(
            parse_main_frame_id(string_id.to_string().as_bytes()),
            Some("ABC123".into())
        );

        let numeric_id = serde_json::json!({
            "frameTree": { "frame": { "id": 42, "url": "about:blank" } }
        });
        assert_eq!(
            parse_main_frame_id(numeric_id.to_string().as_bytes()),
            Some("42".into())
        );
    }

    #[test]
    fn parse_execution_context_id_reads_result_field() {
        let result = serde_json::json!({ "executionContextId": 7 });
        assert_eq!(
            parse_execution_context_id(result.to_string().as_bytes()),
            Some(7)
        );
    }

    #[test]
    fn full_page_to_core_to_page_round_trip() {
        // A demo-style consumer (mirrors register_demo_consumer without the cfg).
        fn pong(msg: &Message) -> Option<serde_json::Value> {
            let ping = msg.data.get("ping").and_then(|v| v.as_str()).unwrap_or("");
            Some(serde_json::json!({ "pong": format!("core-saw:{ping}") }))
        }
        let consumers = vec![Consumer {
            channel: "inject-demo",
            script: "",
            handler: pong,
        }];

        // Simulate what CEF hands us: a bindingCalled event whose payload is the
        // bootstrap's {channel,id,data} envelope.
        let payload = r#"{"channel":"inject-demo","id":11,"data":{"ping":"from-isolated"}}"#;
        let raw = params(SEND_BINDING, payload, 99);

        let call = parse_binding_called(&raw).unwrap();
        assert_eq!(call.execution_context_id, 99);
        let incoming = parse_payload(&call.payload).unwrap();
        let reply = dispatch(&consumers, &incoming, 1).expect("consumer replies");

        // The reply the core would evaluate back into context 99.
        let envelope = serde_json::json!({ "id": incoming.id, "data": reply }).to_string();
        assert!(envelope.contains(r#""pong":"core-saw:from-isolated""#));
        assert!(envelope.contains(r#""id":11"#));
        let expr = receive_expression(&envelope);
        assert!(expr.contains("__receive("));
    }
}
