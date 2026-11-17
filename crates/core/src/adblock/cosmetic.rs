//! Cosmetic ad-block - element-hiding via the injection substrate (P2-21, ADR-0020).
//!
//! Network blocking ([`super::check`]) cancels ad *requests*; cosmetic filtering
//! hides the ad *elements* the network layer can't catch (first-party markup,
//! layout placeholders). It's a [`crate::inject`] consumer: a document-start
//! content script (isolated world) reports its page URL, the core returns the
//! site's element-hiding CSS ([`super::cosmetic_css`]), and the script injects it
//! as a `<style>` - **before the page renders**, so there's no flash (the reason
//! ADR-0020 deferred cosmetic filtering until the substrate existed).
//!
//! **Scope (first cut):** site-specific hide rules (`example.com##.ad`). Generic
//! class/id cosmetics (which need the page's observed class/id names), scriptlets
//! (which need main-world execution), and sub-frame hiding (which needs the
//! all-frames substrate, P2-24) are follow-ups - filed, not silent.

use crate::inject::{register as register_consumer, Consumer, Message};

/// The substrate channel cosmetic messages ride.
const CHANNEL: &str = "cosmetic";

/// Register cosmetic filtering as a substrate consumer. Called once at startup
/// ([`crate::embed::initialize`]), before any browser exists.
pub fn register() {
    register_consumer(Consumer {
        channel: CHANNEL,
        script: INJECT_SCRIPT,
        handler: handle,
    });
}

/// Handle a cosmetic message: return the element-hiding CSS for the page's URL.
/// Runs on the UI thread (via the substrate observer).
pub fn handle(msg: &Message) -> Option<serde_json::Value> {
    match msg.data.get("type").and_then(|v| v.as_str())? {
        "rules" => {
            let url = msg.data.get("url").and_then(|v| v.as_str()).unwrap_or("");
            // Empty string (blocking off / allowlisted / no rules) is a valid,
            // resolving reply - the script just injects nothing.
            Some(serde_json::json!({ "css": super::cosmetic_css(url).unwrap_or_default() }))
        }
        _ => None,
    }
}

/// The content script (isolated world, document-start). Asks the core for the
/// page's element-hiding CSS and injects it as a `<style>`. Appends to
/// `documentElement` because `<head>` may not exist yet at document-start; the
/// stylesheet still applies to the whole document.
const INJECT_SCRIPT: &str = r#"(function () {
  if (!window.__browserCore || document.documentElement.hasAttribute('data-cosmetic-filter')) return;
  document.documentElement.setAttribute('data-cosmetic-filter', '');
  window.__browserCore.send('cosmetic', { type: 'rules', url: location.href })
    .then(function (res) {
      var css = res && res.css;
      if (!css) return;
      var style = document.createElement('style');
      style.setAttribute('data-cosmetic-filter', '');
      style.textContent = css;
      (document.head || document.documentElement).appendChild(style);
    });
})();"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_rules_returns_a_css_field() {
        // No engine installed in the test process → empty CSS, but a well-formed
        // reply that resolves the script's promise.
        let msg = Message {
            channel: CHANNEL,
            data: &serde_json::json!({ "type": "rules", "url": "https://example.com/" }),
            tab_id: 1,
        };
        assert_eq!(handle(&msg).unwrap()["css"], "");
    }

    #[test]
    fn handle_unknown_type_is_none() {
        let msg = Message {
            channel: CHANNEL,
            data: &serde_json::json!({ "type": "frobnicate" }),
            tab_id: 1,
        };
        assert!(handle(&msg).is_none());
    }

    #[test]
    fn inject_script_asks_core_and_injects_a_style() {
        assert!(INJECT_SCRIPT.contains("__browserCore.send('cosmetic'"));
        assert!(INJECT_SCRIPT.contains("location.href"));
        assert!(INJECT_SCRIPT.contains("createElement('style')"));
        // Injects at document-start, tolerant of a missing <head>.
        assert!(INJECT_SCRIPT.contains("document.head || document.documentElement"));
    }
}
