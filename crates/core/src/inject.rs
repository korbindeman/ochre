//! Content-script injection substrate - document-start, isolated world (P2-14,
//! ADR-0017; WebKit since ADR-0072).
//!
//! The *one hard thing built once* that autofill, cosmetic ad-block,
//! cookie-consent, and theme-color sampling all ride. It gives the core two
//! capabilities against any page:
//!
//! 1. **Run a content script at document-start in an isolated world** - a
//!    JavaScript world that shares the page's DOM but *not* its globals, so page
//!    JS can neither reach nor forge our bridge, and our script runs before the
//!    page's own scripts.
//! 2. **A page↔core message channel with replies** - the script asks the core
//!    for data and the core answers; credentials and rules never live in the
//!    injected source.
//!
//! ## How it works
//!
//! Every profile's `WKUserContentController` carries, in the named content world
//! [`ISOLATED_WORLD`]: the bootstrap plus each consumer's script as
//! document-start `WKUserScript`s for **all frames** (cross-origin iframes
//! included, which the CEF version could not reach), and a reply-capable script
//! message handler. The bootstrap's `send` posts `{channel, id, data}` to that
//! handler and resolves with the reply WebKit returns, so request and response
//! need no correlation of our own.
//!
//! ## Registry
//!
//! Consumers [`register`] a `{channel, script, handler}` at startup, before any
//! web view exists.

use std::sync::Mutex;

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
use objc2_foundation::NSString;
use objc2_web_kit::{
    WKContentWorld, WKScriptMessage, WKScriptMessageHandlerWithReply, WKUserContentController,
    WKUserScript, WKUserScriptInjectionTime,
};

/// Name of the content world our scripts and message handler live in.
const ISOLATED_WORLD: &str = "BrowserCoreWorld";

/// Script-message handler name. It exists only in the isolated world.
const HANDLER_NAME: &str = "__browser_core";

// ─── Public consumer API ─────────────────────────────────────────────────

/// A message from an isolated-world content script, routed to the consumer that
/// owns its `channel`.
pub struct Message<'a> {
    /// The consumer channel the script addressed.
    pub channel: &'a str,
    /// The `data` payload the script sent (arbitrary JSON; `Null` if omitted).
    pub data: &'a serde_json::Value,
    /// The web view id of the tab the message came from, for consumers that
    /// key per-tab state (e.g. consent's rejected-banner count).
    pub tab_id: i32,
}

/// A consumer's message handler. Runs on the main thread. Returns `Some(reply)`
/// to answer the script (resolving its `send(...)` promise), or `None` for
/// fire-and-forget (the promise resolves with `undefined`).
pub type Handler = fn(&Message) -> Option<serde_json::Value>;

/// A registered rider on the substrate: a document-start `script` (isolated
/// world) plus the `handler` for the messages it sends on `channel`.
///
/// The script is `&'static str` on purpose: the injected source is *generic*
/// and must never carry per-page secrets - those cross only via the message
/// channel, on demand (ADR-0017).
pub struct Consumer {
    pub channel: &'static str,
    pub script: &'static str,
    pub handler: Handler,
}

static CONSUMERS: Mutex<Vec<Consumer>> = Mutex::new(Vec::new());

fn consumers() -> std::sync::MutexGuard<'static, Vec<Consumer>> {
    CONSUMERS.lock().unwrap_or_else(|p| p.into_inner())
}

/// Register a consumer. Call at startup, before any web view is created.
pub fn register(consumer: Consumer) {
    consumers().push(consumer);
}

// ─── Installation ────────────────────────────────────────────────────────

fn isolated_world(mtm: MainThreadMarker) -> Retained<WKContentWorld> {
    // SAFETY: main thread; the named world is created once and shared.
    unsafe { WKContentWorld::worldWithName(&NSString::from_str(ISOLATED_WORLD), mtm) }
}

/// Add the bootstrap and every consumer's script to `content`.
pub(crate) fn add_scripts(content: &WKUserContentController, mtm: MainThreadMarker) {
    let mut sources = vec![BOOTSTRAP_JS.to_string()];
    sources.extend(consumers().iter().map(|c| c.script.to_string()));
    let world = isolated_world(mtm);
    for source in sources {
        // SAFETY: main thread; document start, all frames, isolated world.
        unsafe {
            let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                mtm.alloc(),
                &NSString::from_str(&source),
                WKUserScriptInjectionTime::AtDocumentStart,
                false,
                &world,
            );
            content.addUserScript(&script);
        }
    }
}

/// Add the page→core message handler to `content` (once per controller).
pub(crate) fn add_handler(content: &WKUserContentController, mtm: MainThreadMarker) {
    let handler: Retained<MessageHandler> = unsafe { msg_send![MessageHandler::alloc(mtm), init] };
    // SAFETY: main thread; the controller retains the handler.
    unsafe {
        content.addScriptMessageHandlerWithReply_contentWorld_name(
            ProtocolObject::from_ref(&*handler),
            &isolated_world(mtm),
            &NSString::from_str(HANDLER_NAME),
        );
    }
}

/// Evaluate `expression` in `browser`'s page world (the explicit-gesture vault
/// fill, ADR-0029). The page can see this script, which is why only a user
/// action triggers it.
pub fn evaluate_main_world(browser: &crate::engine::Browser, expression: &str) {
    browser.evaluate_main_world(expression);
}

// ─── Page → core ─────────────────────────────────────────────────────────

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OchreInjectHandler"]
    struct MessageHandler;

    unsafe impl NSObjectProtocol for MessageHandler {}

    unsafe impl WKScriptMessageHandlerWithReply for MessageHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:replyHandler:))]
        fn did_receive(
            &self,
            _content: &WKUserContentController,
            message: &WKScriptMessage,
            reply: &DynBlock<dyn Fn(*mut AnyObject, *mut NSString)>,
        ) {
            // SAFETY: property reads on the message, main thread.
            let body = unsafe { message.body() };
            let tab_id = unsafe { message.webView() }
                .and_then(|view| crate::engine::browser_from_view(&view))
                .unwrap_or(-1);
            let answer = body
                .downcast_ref::<NSString>()
                .and_then(|text| parse_payload(&text.to_string()))
                .and_then(|incoming| dispatch(&consumers(), &incoming, tab_id));
            match answer {
                Some(value) => {
                    let json = NSString::from_str(&value.to_string());
                    reply.call((
                        Retained::as_ptr(&json).cast_mut().cast(),
                        std::ptr::null_mut(),
                    ));
                }
                None => reply.call((std::ptr::null_mut(), std::ptr::null_mut())),
            }
        }
    }
);

/// A parsed page message: `{channel, id, data}`.
struct Incoming {
    channel: String,
    data: serde_json::Value,
}

/// Parse the JSON payload the bootstrap sends.
fn parse_payload(payload: &str) -> Option<Incoming> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    Some(Incoming {
        channel: v.get("channel")?.as_str()?.to_string(),
        data: v.get("data").cloned().unwrap_or(serde_json::Value::Null),
    })
}

/// Pure dispatch: find the consumer for `incoming.channel` and run its handler.
fn dispatch(consumers: &[Consumer], incoming: &Incoming, tab_id: i32) -> Option<serde_json::Value> {
    let consumer = consumers.iter().find(|c| c.channel == incoming.channel)?;
    (consumer.handler)(&Message {
        channel: &incoming.channel,
        data: &incoming.data,
        tab_id,
    })
}

// ─── The bootstrap script (isolated world, document-start) ───────────────

/// Runs first in the isolated world of every frame. Installs
/// `window.__browserCore.send(channel, data)` → a Promise resolved with the
/// core's reply (JSON-decoded), or `undefined` for no reply. Idempotent.
const BOOTSTRAP_JS: &str = r#"(function () {
  if (window.__browserCore) return;
  var handler = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.__browser_core;
  var seq = 0;
  window.__browserCore = {
    send: function (channel, data) {
      if (!handler) return Promise.resolve(undefined);
      var msg = JSON.stringify({ channel: channel, id: ++seq, data: data });
      return handler.postMessage(msg).then(function (reply) {
        if (reply === null || reply === undefined) return undefined;
        try { return JSON.parse(reply); } catch (e) { return undefined; }
      }, function () { return undefined; });
    }
  };
})();"#;

// ─── Debug-only demo consumer (live round-trip proof) ────────────────────

/// Register a demo consumer that exercises the full round-trip against a real
/// page. Debug builds only, gated on `BROWSER_INJECT_DEMO`.
#[cfg(debug_assertions)]
pub fn register_demo_consumer() {
    register(Consumer {
        channel: "inject-demo",
        script: DEMO_SCRIPT,
        handler: demo_handler,
    });
}

/// Echoes the page's `ping` back as `pong`, prefixed so the reply is provably
/// core-authored.
#[cfg(debug_assertions)]
fn demo_handler(msg: &Message) -> Option<serde_json::Value> {
    let ping = msg.data.get("ping").and_then(|v| v.as_str()).unwrap_or("");
    Some(serde_json::json!({ "pong": format!("core-saw:{ping}") }))
}

/// Writes the reply to a DOM attribute (the DOM is shared with the page world),
/// while `window.__browserCore` stays invisible to the page.
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
            data: serde_json::json!({ "x": 1 }),
        };
        assert_eq!(
            dispatch(&consumers, &hit, 7),
            Some(serde_json::json!({ "saw": { "x": 1 } }))
        );
        let miss = Incoming {
            channel: "nope".into(),
            data: serde_json::Value::Null,
        };
        assert!(dispatch(&consumers, &miss, 7).is_none());
    }

    #[test]
    fn parse_payload_reads_channel_and_data() {
        let incoming =
            parse_payload(r#"{"channel":"autofill","id":3,"data":{"host":"example.com"}}"#)
                .expect("valid payload");
        assert_eq!(incoming.channel, "autofill");
        assert_eq!(incoming.data["host"], "example.com");
        let no_data = parse_payload(r#"{"channel":"c","id":1}"#).expect("data optional");
        assert!(no_data.data.is_null());
        assert!(parse_payload(r#"{"id":1}"#).is_none());
        assert!(parse_payload("garbage").is_none());
    }

    #[test]
    fn bootstrap_uses_the_isolated_handler_name() {
        assert!(BOOTSTRAP_JS.contains(HANDLER_NAME));
    }
}
