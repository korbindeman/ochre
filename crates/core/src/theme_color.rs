//! Page theme-color sampling for the viewport URL bar (Safari-style chrome).
//!
//! A lightweight injection-substrate consumer reads `meta[name="theme-color"]`
//! (falling back to the document background) and streams updates to the host so
//! the floating address bar can tint itself to match the page.

use crate::inject::{register as register_consumer, Consumer, Message};

const CHANNEL: &str = "theme-color";

/// Register the theme-color consumer at startup (before any browser exists).
pub fn register() {
    register_consumer(Consumer {
        channel: CHANNEL,
        script: SCRIPT,
        handler: handle,
    });
}

fn handle(msg: &Message) -> Option<serde_json::Value> {
    let color = msg.data.get("color").and_then(|v| v.as_str()).unwrap_or("");
    if crate::private_window::note_theme_color(msg.tab_id, color) {
        return None;
    }
    crate::tabs::note_theme_color(msg.tab_id, color);
    None
}

/// Runs in the isolated world at document-start. Reports whenever the sampled
/// color changes — meta tag edits, late body paint, SPA navigations.
const SCRIPT: &str = r#"(function () {
  if (!window.__browserCore) return;
  var last = '';
  function normalize(c) {
    return (c || '').trim();
  }
  function read() {
    var meta = document.querySelector('meta[name="theme-color"]');
    if (meta && meta.content) return normalize(meta.content);
    var body = document.body;
    if (body) {
      var bg = getComputedStyle(body).backgroundColor;
      if (bg && bg !== 'rgba(0, 0, 0, 0)' && bg !== 'transparent') return bg;
    }
    var htmlBg = getComputedStyle(document.documentElement).backgroundColor;
    if (htmlBg && htmlBg !== 'rgba(0, 0, 0, 0)' && htmlBg !== 'transparent') {
      return htmlBg;
    }
    return '';
  }
  function report() {
    var color = read();
    if (color === last) return;
    last = color;
    window.__browserCore.send('theme-color', { color: color });
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', report);
  } else {
    report();
  }
  window.addEventListener('load', report);
  new MutationObserver(report).observe(document.documentElement, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['content', 'style', 'class']
  });
})();"#;
