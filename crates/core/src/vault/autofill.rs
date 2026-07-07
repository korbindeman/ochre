//! Autofill — fill login forms from the unlocked vault (P2-15/P2-16, ADR-0017).
//!
//! Autofill is a **consumer of the injection substrate** ([`crate::inject`]): a
//! generic content script runs at document-start in the isolated world, detects
//! the login form once the DOM is ready, and *asks the core* what to do. The
//! script never carries a credential — it carries only detection + UI logic — and
//! the core answers over the substrate's message channel:
//!
//! - `{type:"lookup", host}` → `{matches:[{id, username, name}]}` —
//!   [`crate::vault::service::lookup`] metadata **only, never a password**.
//! - `{type:"password", id}` → `{password}` — the single seam a secret crosses,
//!   requested **only when the user picks an entry** in the inline menu — an
//!   explicit gesture, so a page's autofocus can't silently pull a password out.
//!
//! ## The picker (P2-16)
//!
//! Focusing a login field (under a real user activation) opens an **in-page
//! inline menu** drawn by the content script (ADR-0017 chose in-page over a native
//! popover: no web-view→screen coordinate mapping, works on every frontend). It is
//! rendered in an **open shadow root** so the page's CSS can't restyle it, lists
//! the matching logins, supports keyboard (↑/↓/↵/esc) and click, and fills only
//! the entry the user selects. This supersedes P2-15's interim focus-auto-fill:
//! the password now crosses on the *selection* gesture, not on focus.
//!
//! The **save-password banner** is native (Safari-style) and split to P2-23 — it
//! needs the M2 chrome. Detection is continuous (P2-28/29): a debounced
//! `MutationObserver` + SPA-navigation hooks re-scan for fields injected after
//! load, pierce open shadow roots, and offer on username-only steps (Google's
//! email screen) as well as full login forms.

use crate::inject::{register as register_consumer, Consumer, Message};
use crate::vault::service::{self, LoginView};

/// The substrate channel autofill messages ride.
const CHANNEL: &str = "autofill";

/// Register autofill as a substrate consumer. Called once at startup
/// ([`crate::embed::initialize`]), before any browser exists. Unconditional: the
/// detect script is cheap and simply finds no matches while the vault is locked.
pub fn register() {
    register_consumer(Consumer {
        channel: CHANNEL,
        script: DETECT_SCRIPT,
        handler: handle,
    });
}

/// Handle an autofill message from an isolated-world content script. Runs on the
/// UI thread (via the substrate observer). Returns the reply the script awaits.
pub fn handle(msg: &Message) -> Option<serde_json::Value> {
    match msg.data.get("type").and_then(|v| v.as_str())? {
        "lookup" => {
            let host = msg.data.get("host").and_then(|v| v.as_str()).unwrap_or("");
            Some(lookup_reply(service::lookup(host)))
        }
        // The one path a secret crosses — gated on the page's explicit gesture
        // (the script only sends this when the user selects a picker entry).
        "password" => {
            let id = msg.data.get("id").and_then(|v| v.as_str()).unwrap_or("");
            Some(password_reply(service::password_for(id)))
        }
        _ => None,
    }
}

/// Shape a lookup reply: match **metadata only** (id + username + display name),
/// never a password.
fn lookup_reply(matches: Vec<LoginView>) -> serde_json::Value {
    let matches: Vec<serde_json::Value> = matches
        .into_iter()
        .map(|l| serde_json::json!({ "id": l.id, "username": l.username, "name": l.name }))
        .collect();
    serde_json::json!({ "matches": matches })
}

/// Shape a password reply. `None` (locked / unknown id) serializes to `null`, so
/// the script's awaiting promise always resolves rather than hanging.
fn password_reply(password: Option<String>) -> serde_json::Value {
    serde_json::json!({ "password": password })
}

/// The content script (isolated world, document-start). Continuously scans for
/// login fields (debounced `MutationObserver` + SPA-navigation hooks, shadow-DOM
/// piercing), asks the core for matches, and on gesture-focus opens an inline
/// picker (shadow DOM). Selecting an entry requests that login's password and
/// fills via the native value setter + a framework-compatible event sequence.
const DETECT_SCRIPT: &str = r##"(function () {
  if (!window.__browserCore || window.__vaultAutofill) return;
  window.__vaultAutofill = true;

  var HOST_ID = '__vault_autofill_picker';
  var SCAN_MS = 200;
  var activePicker = null;
  var scanTimer = null;
  var lastUrl = location.href;
  var wired = typeof WeakSet !== 'undefined' ? new WeakSet() : null;

  function visible(el) {
    return el.offsetParent !== null || el.getClientRects().length > 0;
  }
  function loginCandidate(el) {
    if (!el || el.disabled || el.readOnly || el.type === 'hidden') return false;
    if (el.type === 'search') return false;
    var ac = (el.getAttribute('autocomplete') || '').toLowerCase();
    if (ac === 'off' || ac.indexOf('search') >= 0) return false;
    if (el.getAttribute('role') === 'searchbox') return false;
    return true;
  }
  // Pierce open shadow roots when scanning (P2-28).
  function allInputs(root) {
    root = root || document;
    var out = [];
    try {
      var nodes = root.querySelectorAll('input');
      for (var i = 0; i < nodes.length; i++) out.push(nodes[i]);
    } catch (e) {}
    var els = root.querySelectorAll ? root.querySelectorAll('*') : [];
    for (var j = 0; j < els.length; j++) {
      if (els[j].shadowRoot) out = out.concat(allInputs(els[j].shadowRoot));
    }
    return out;
  }
  function fieldHint(el) {
    return ((el.getAttribute('autocomplete') || '') + ' ' + (el.name || '') + ' ' +
      (el.id || '') + ' ' + (el.getAttribute('aria-label') || '')).toLowerCase();
  }
  function userScore(el) {
    var ac = (el.getAttribute('autocomplete') || '').toLowerCase();
    if (ac.indexOf('username') >= 0) return 100;
    if (ac === 'email') return 90;
    if (el.type === 'email') return 80;
    var h = fieldHint(el);
    if (/email|user|login|identifier|account/.test(h)) return 70;
    if (el.type === 'tel') return 50;
    if (el.type === 'text' || !el.type) return 30;
    return 0;
  }
  function pwScore(el) {
    var ac = (el.getAttribute('autocomplete') || '').toLowerCase();
    if (ac.indexOf('current-password') >= 0) return 100;
    if (ac.indexOf('new-password') >= 0) return 90;
    if (el.type === 'password') return 80;
    var h = fieldHint(el);
    if (/pass|pwd|password/.test(h)) return 70;
    return 0;
  }
  function bestOf(list, score) {
    var best = null, bestS = 0;
    for (var i = 0; i < list.length; i++) {
      var s = score(list[i]);
      if (s > bestS) { bestS = s; best = list[i]; }
    }
    return best;
  }
  // Classify login fields — username-only, password-only, or both (P2-29).
  function findFields() {
    var inputs = allInputs().filter(visible).filter(loginCandidate);
    var users = [], pws = [];
    for (var i = 0; i < inputs.length; i++) {
      var el = inputs[i];
      if (el.type === 'password' || pwScore(el) >= 70) pws.push(el);
      else if (userScore(el) > 0) users.push(el);
    }
    var pw = bestOf(pws, pwScore);
    var user = null;
    if (pw) {
      var before = users.filter(function (u) {
        return pw.compareDocumentPosition(u) & Node.DOCUMENT_POSITION_PRECEDING;
      });
      user = bestOf(before, userScore) || before[before.length - 1] || null;
    } else if (users.length) {
      user = bestOf(users, userScore);
    }
    if (!user && !pw) return null;
    return { user: user, pw: pw };
  }
  // Framework-compatible fill sequence (P2-30).
  function setValue(el, v) {
    if (!el) return;
    try { el.focus(); } catch (e) {}
    el.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true }));
    var d = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), 'value');
    if (d && d.set) { d.set.call(el, v); } else { el.value = v; }
    try {
      el.dispatchEvent(new InputEvent('beforeinput', {
        bubbles: true, cancelable: true, inputType: 'insertText', data: v
      }));
    } catch (e) {}
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new KeyboardEvent('keyup', { bubbles: true, cancelable: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    try { el.blur(); } catch (e) {}
  }
  function userActivated() {
    return !navigator.userActivation || navigator.userActivation.hasBeenActive;
  }

  var CSS =
    ':host{all:initial}' +
    '.menu{position:fixed;font:13px -apple-system,system-ui,sans-serif;' +
    'background:rgb(250,250,250);color:rgb(20,20,20);border:1px solid rgba(0,0,0,.12);' +
    'border-radius:10px;box-shadow:0 6px 24px rgba(0,0,0,.20);padding:4px;' +
    'min-width:220px;max-width:360px;box-sizing:border-box;overflow:hidden;z-index:2147483647}' +
    '.row{display:flex;flex-direction:column;gap:1px;padding:7px 10px;border-radius:6px;cursor:default}' +
    '.row[aria-selected=true]{background:rgb(10,110,240);color:white}' +
    '.u{font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}' +
    '.n{font-size:11px;opacity:.6}' +
    '.row[aria-selected=true] .n{opacity:.85}' +
    '@media (prefers-color-scheme:dark){.menu{background:rgb(44,44,46);color:rgb(240,240,240);' +
    'border-color:rgba(255,255,255,.12)}}';

  function Picker(fields, matches) {
    this.fields = fields;
    this.matches = matches;
    this.index = 0;
    this.host = null;
    this.root = null;
    this.menu = null;
    this.anchor = null;
    var self = this;
    this.onScroll = function () { if (self.host) self.position(); };
    this.onDown = function (e) {
      if (self.host && e.composedPath && e.composedPath().indexOf(self.menu) === -1) {
        self.dismiss();
      }
    };
    this.onKey = function (e) { self.key(e); };
  }
  Picker.prototype.open = function (anchor) {
    if (!userActivated()) return;
    this.anchor = anchor;
    if (this.host) { this.position(); return; }
    var host = document.createElement('div');
    host.id = HOST_ID;
    var root = host.attachShadow({ mode: 'open' });
    var style = document.createElement('style');
    style.textContent = CSS;
    var menu = document.createElement('div');
    menu.className = 'menu';
    menu.setAttribute('role', 'listbox');
    for (var i = 0; i < this.matches.length; i++) {
      menu.appendChild(this.rowEl(this.matches[i], i));
    }
    root.appendChild(style);
    root.appendChild(menu);
    document.documentElement.appendChild(host);
    this.host = host; this.root = root; this.menu = menu;
    this.select(0);
    this.position();
    window.addEventListener('scroll', this.onScroll, true);
    window.addEventListener('resize', this.onScroll, true);
    document.addEventListener('pointerdown', this.onDown, true);
    this.anchor.addEventListener('keydown', this.onKey, true);
  };
  Picker.prototype.rowEl = function (m, i) {
    var self = this;
    var row = document.createElement('div');
    row.className = 'row';
    row.setAttribute('role', 'option');
    row.setAttribute('data-idx', String(i));
    if (m.username) row.setAttribute('data-user', m.username);
    var u = document.createElement('div');
    u.className = 'u';
    u.textContent = m.username || '(no username)';
    var n = document.createElement('div');
    n.className = 'n';
    n.textContent = m.name || '';
    row.appendChild(u);
    if (m.name) row.appendChild(n);
    row.addEventListener('mousemove', function () { self.select(i); });
    row.addEventListener('mousedown', function (e) { e.preventDefault(); });
    row.addEventListener('click', function () { self.choose(i); });
    return row;
  };
  Picker.prototype.select = function (i) {
    this.index = i;
    var rows = this.menu.querySelectorAll('.row');
    for (var j = 0; j < rows.length; j++) {
      rows[j].setAttribute('aria-selected', j === i ? 'true' : 'false');
    }
  };
  Picker.prototype.key = function (e) {
    if (!this.host) return;
    if (e.key === 'ArrowDown') { e.preventDefault(); this.select((this.index + 1) % this.matches.length); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); this.select((this.index - 1 + this.matches.length) % this.matches.length); }
    else if (e.key === 'Enter') { e.preventDefault(); this.choose(this.index); }
    else if (e.key === 'Escape') { e.preventDefault(); this.dismiss(); }
  };
  Picker.prototype.choose = function (i) {
    var self = this;
    var m = this.matches[i];
    window.__browserCore.send('autofill', { type: 'password', id: m.id }).then(function (r) {
      self.dismiss();
      var pw = r && r.password;
      if (m.username && self.fields.user) setValue(self.fields.user, m.username);
      if (pw && self.fields.pw) setValue(self.fields.pw, pw);
    });
  };
  Picker.prototype.position = function () {
    var r = this.anchor.getBoundingClientRect();
    var vh = window.innerHeight;
    this.menu.style.left = Math.round(r.left) + 'px';
    this.menu.style.minWidth = Math.round(r.width) + 'px';
    var below = vh - r.bottom;
    var mh = this.menu.offsetHeight;
    if (below < mh + 8 && r.top > below) {
      this.menu.style.top = Math.round(r.top - mh - 4) + 'px';
    } else {
      this.menu.style.top = Math.round(r.bottom + 4) + 'px';
    }
  };
  Picker.prototype.dismiss = function () {
    if (!this.host) return;
    window.removeEventListener('scroll', this.onScroll, true);
    window.removeEventListener('resize', this.onScroll, true);
    document.removeEventListener('pointerdown', this.onDown, true);
    if (this.anchor) this.anchor.removeEventListener('keydown', this.onKey, true);
    this.host.remove();
    this.host = this.root = this.menu = null;
  };

  function wireField(el) {
    if (!el) return;
    if (wired && wired.has(el)) return;
    if (wired) wired.add(el);
    el.addEventListener('focusin', function () {
      if (activePicker) activePicker.open(el);
    });
  }
  function teardownPicker() {
    if (activePicker) { activePicker.dismiss(); activePicker = null; }
  }
  function setupPicker(fields, matches) {
    teardownPicker();
    activePicker = new Picker(fields, matches);
    wireField(fields.user);
    wireField(fields.pw);
  }
  function scan() {
    if (location.href !== lastUrl) lastUrl = location.href;
    var fields = findFields();
    if (!fields) { teardownPicker(); return; }
    window.__browserCore.send('autofill', { type: 'lookup', host: location.hostname })
      .then(function (res) {
        var matches = (res && res.matches) || [];
        if (!matches.length) { teardownPicker(); return; }
        setupPicker(fields, matches);
      });
  }
  function scheduleScan() {
    if (scanTimer) clearTimeout(scanTimer);
    scanTimer = setTimeout(scan, SCAN_MS);
  }
  function onNav() {
    if (location.href === lastUrl) return;
    lastUrl = location.href;
    teardownPicker();
    scheduleScan();
  }

  // SPA same-document navigations (P2-28).
  var _push = history.pushState;
  var _replace = history.replaceState;
  history.pushState = function () { _push.apply(this, arguments); onNav(); };
  history.replaceState = function () { _replace.apply(this, arguments); onNav(); };
  window.addEventListener('popstate', onNav);

  // Continuous re-detection for late-injected forms (P2-28).
  try {
    new MutationObserver(scheduleScan).observe(document.documentElement, {
      childList: true, subtree: true, attributes: true, attributeFilter: ['type', 'autocomplete']
    });
  } catch (e) {}

  function boot() { scheduleScan(); }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot);
  } else {
    boot();
  }
})();"##;

// ─── Button-driven fill (P2-34, ADR-0029) ────────────────────────────────
//
// The URL-bar vault button (native) fills the active tab by picking a login and
// having the core *push* this snippet into the page — the core-initiated path
// ([`crate::inject::evaluate_main_world`]), vs. the focus-driven picker above
// which the page pulls. It is deliberately **lenient**: unlike [`DETECT_SCRIPT`]
// (which needs a password field before it offers anything), it fills whatever
// login fields are present — so it works on a two-step form's **email-only first
// step** (Google) as well as a full user+password form. It runs in the main world
// (a fill exposes the value to the page regardless), so it carries its own tiny
// `setVal` rather than reaching the isolated world.

/// The fill routine, invoked by [`fill_expression`] with the username (or `null`)
/// and password as JS string literals.
const FILL_FN: &str = r##"(function (u, p) {
  function vis(el) { return el && (el.offsetParent !== null || el.getClientRects().length > 0); }
  function setVal(el, v) {
    if (!el) return;
    try { el.focus(); } catch (e) {}
    el.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true }));
    var d = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), 'value');
    if (d && d.set) { d.set.call(el, v); } else { el.value = v; }
    try {
      el.dispatchEvent(new InputEvent('beforeinput', {
        bubbles: true, cancelable: true, inputType: 'insertText', data: v
      }));
    } catch (e) {}
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new KeyboardEvent('keyup', { bubbles: true, cancelable: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    try { el.blur(); } catch (e) {}
  }
  var all = function (s) { return Array.prototype.slice.call(document.querySelectorAll(s)); };
  var USER = 'input[autocomplete=username],input[type=email],input[type=text],input[type=tel],input:not([type])';
  // Prefer a field that self-identifies as a username/email (autocomplete / name /
  // id / type) over a bare text input, so a stray search box loses to the real one.
  function prefer(list) {
    return list.filter(function (e) {
      return /user|email|login/i.test((e.getAttribute('autocomplete') || '') + ' ' + (e.name || '') + ' ' + (e.id || ''));
    })[0] || list.filter(function (e) { return e.type === 'email'; })[0];
  }
  var pw = all('input[type=password]').filter(vis)[0] || null;
  var user;
  if (pw) {
    // Username = the best text-ish field *before* the password.
    var before = all(USER).filter(vis).filter(function (i) {
      return pw.compareDocumentPosition(i) & Node.DOCUMENT_POSITION_PRECEDING;
    });
    user = prefer(before) || before[before.length - 1] || null;
  } else {
    // No password yet (step one): the best lone identifier field.
    var cands = all(USER).filter(vis);
    user = prefer(cands) || cands[0] || null;
  }
  if (u && user) setVal(user, u);
  if (p && pw) setVal(pw, p);
})"##;

/// Build the main-world fill expression for a credential — [`FILL_FN`] invoked
/// with the username (or `null`) and password as **safe JS string literals**
/// (JSON is valid JS; the same escaping the substrate's reply path uses, so a
/// quote/backslash in a password can't break out of the literal).
pub fn fill_expression(username: Option<&str>, password: &str) -> String {
    let user = username
        .map(|u| serde_json::to_string(u).unwrap_or_else(|_| "null".into()))
        .unwrap_or_else(|| "null".into());
    let pw = serde_json::to_string(password).unwrap_or_else(|_| "\"\"".into());
    format!("{FILL_FN}({user},{pw});")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(id: &str, username: Option<&str>, name: &str) -> LoginView {
        LoginView {
            id: id.to_string(),
            name: name.to_string(),
            username: username.map(str::to_string),
            uris: vec!["https://example.com".to_string()],
        }
    }

    #[test]
    fn lookup_reply_is_metadata_only_with_display_name() {
        let reply = lookup_reply(vec![
            view("a", Some("alice"), "Example"),
            view("b", None, "Other"),
        ]);
        assert_eq!(reply["matches"][0]["id"], "a");
        assert_eq!(reply["matches"][0]["username"], "alice");
        assert_eq!(reply["matches"][0]["name"], "Example");
        assert_eq!(reply["matches"][1]["id"], "b");
        assert!(reply["matches"][1]["username"].is_null());
        // Crucial: the metadata reply carries no password field at all.
        assert!(
            !serde_json::to_string(&reply).unwrap().contains("password"),
            "lookup reply must never mention a password: {reply}"
        );
    }

    #[test]
    fn lookup_reply_empty_when_no_matches() {
        assert_eq!(lookup_reply(vec![])["matches"], serde_json::json!([]));
    }

    #[test]
    fn password_reply_resolves_to_null_when_absent() {
        // A locked/unknown result must still resolve the script's promise (null),
        // not omit the key (which would hang the await).
        assert_eq!(
            password_reply(None),
            serde_json::json!({ "password": null })
        );
        assert_eq!(
            password_reply(Some("s3cret".into())),
            serde_json::json!({ "password": "s3cret" })
        );
    }

    #[test]
    fn handle_locked_vault_returns_empty_matches_and_null_password() {
        // No vault initialized in this test process → locked.
        let lookup = Message {
            channel: CHANNEL,
            data: &serde_json::json!({ "type": "lookup", "host": "example.com" }),
        };
        assert_eq!(handle(&lookup).unwrap()["matches"], serde_json::json!([]));

        let password = Message {
            channel: CHANNEL,
            data: &serde_json::json!({ "type": "password", "id": "whatever" }),
        };
        assert!(handle(&password).unwrap()["password"].is_null());
    }

    #[test]
    fn handle_unknown_or_missing_type_is_none() {
        let unknown = Message {
            channel: CHANNEL,
            data: &serde_json::json!({ "type": "frobnicate" }),
        };
        assert!(handle(&unknown).is_none());
        let missing = Message {
            channel: CHANNEL,
            data: &serde_json::json!({ "host": "example.com" }),
        };
        assert!(handle(&missing).is_none());
    }

    #[test]
    fn fill_expression_escapes_and_is_lenient() {
        // Username + password become safe JS string literals; a quote in the
        // password can't break out of the call.
        let expr = fill_expression(Some("alice"), r#"a"b\c"#);
        assert!(expr.contains(r#"("alice","a\"b\\c");"#), "got: {expr}");
        // No username → the first arg is the literal `null` (fill password only).
        let no_user = fill_expression(None, "pw");
        assert!(no_user.contains("(null,\"pw\");"), "got: {no_user}");
        // Lenient: fills even without a password field (the two-step first step),
        // and prefers a self-identifying username field over a bare text box.
        assert!(FILL_FN.contains("No password yet"));
        assert!(FILL_FN.contains("input[type=password]"));
        assert!(FILL_FN.contains("prefer"));
    }

    #[test]
    fn detect_script_is_isolated_gesture_gated_picker() {
        // Rides the substrate bridge, not the page's globals.
        assert!(DETECT_SCRIPT.contains("window.__browserCore"));
        // Asks the core by origin; never embeds a credential in the source.
        assert!(DETECT_SCRIPT.contains("location.hostname"));
        assert!(!DETECT_SCRIPT.contains("password:")); // no baked-in secret
                                                       // Gesture-gated: guarded by user activation.
        assert!(DETECT_SCRIPT.contains("userActivation"));
        // Continuous detection (P2-28/29): SPA hooks + MutationObserver, not one-shot.
        assert!(DETECT_SCRIPT.contains("MutationObserver"));
        assert!(DETECT_SCRIPT.contains("pushState"));
        assert!(DETECT_SCRIPT.contains("shadowRoot"));
        // Username-only steps (Google email screen) don't require a password field.
        assert!(DETECT_SCRIPT.contains("users.length"));
        // Framework-compatible fill (P2-30).
        assert!(DETECT_SCRIPT.contains("beforeinput"));
        // The picker: a shadow-root inline menu, keyboard-navigable.
        assert!(DETECT_SCRIPT.contains("attachShadow"));
        assert!(DETECT_SCRIPT.contains("ArrowDown"));
        // The password is requested from `choose` (selection), not on focus.
        let choose = DETECT_SCRIPT
            .find("choose = function")
            .expect("choose exists");
        let pw_req = DETECT_SCRIPT
            .find("type: 'password'")
            .expect("password request exists");
        assert!(
            pw_req > choose,
            "the password request must live in the selection path, not focus"
        );
        // Vault data is written via textContent, never innerHTML.
        assert!(DETECT_SCRIPT.contains("textContent"));
        assert!(!DETECT_SCRIPT.contains("innerHTML"));
    }
}
