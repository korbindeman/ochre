//! Cookie-consent — reject-first CMP automation + banner hiding (P2-22/P2-25/P2-26, ADR-0026).
//!
//! EU consent banners (GDPR/ePrivacy CMPs) are handled by *opting out*, not just
//! hiding — hiding-without-rejecting leaves the user tracked behind a clean face,
//! which is the anti-pattern we position against (ADR-0026). This [`crate::inject`]
//! consumer runs both tiers, gated on one **cookie-consent toggle** (distinct from
//! ad-block):
//!
//! - **Tier B (reject)** — detect a known CMP and click its "reject all" control,
//!   so the CMP dismisses itself with consent *denied* (OneTrust/Cookiebot/Didomi
//!   and other top CMPs via a core-served ruleset; a `MutationObserver` catches
//!   async banners).
//! - **Tier A (hide)** — a consent-plugin stylesheet *plus* the EasyList Cookie
//!   cosmetic engine (bundled seed → background refresh → hot-swap). Generic
//!   `##` rules are matched by scanning the page's class/id names and injecting
//!   `display:none` CSS at document-start. Honest fallback: hidden ≠ rejected.

mod lists;

use crate::adblock::build_engine;
use crate::inject::{register as register_consumer, Consumer, Message};
use adblock::Engine;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// The substrate channel cookie-consent messages ride.
const CHANNEL: &str = "consent";

/// Per-tab count of consent banners rejected since the tab's last main-frame
/// navigation, keyed by CEF browser identifier. Feeds the shield popover
/// (mirrors [`crate::adblock`]'s blocked-request counter).
static COUNTS: OnceLock<Mutex<HashMap<i32, u64>>> = OnceLock::new();

fn counts() -> &'static Mutex<HashMap<i32, u64>> {
    COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record a rejected consent banner against a tab.
fn note_rejected(tab_id: i32) {
    *counts()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(tab_id)
        .or_insert(0) += 1;
}

/// Rejected-banner count for a tab since its last main-frame navigation.
pub fn rejected_count(tab_id: i32) -> u64 {
    counts()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&tab_id)
        .copied()
        .unwrap_or(0)
}

/// Reset a tab's count — call on a main-frame navigation start (new page).
pub fn reset_count(tab_id: i32) {
    counts()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(tab_id, 0);
}

/// Drop a tab's counter when the tab closes, so the map doesn't grow unbounded.
pub fn forget_tab(tab_id: i32) {
    counts()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&tab_id);
}

// ─── Cookie-list engine (Tier A cosmetics, P2-26) ─────────────────────────

static ENGINE: OnceLock<RwLock<Arc<Engine>>> = OnceLock::new();

/// Install the cookie-list engine (seed on first call, hot-swap on refresh).
pub(crate) fn install_engine(rules: &str) {
    let engine = Arc::new(build_engine(rules));
    match ENGINE.get() {
        Some(lock) => *lock.write().unwrap_or_else(|p| p.into_inner()) = engine,
        None => {
            let _ = ENGINE.set(RwLock::new(engine));
        }
    }
}

/// Bring up the cookie-list engine: seed synchronously, full list in background.
pub fn init_lists() {
    install_engine(lists::SEED);
    lists::load_and_refresh();
}

/// Fold hide selectors into injectable CSS. Empty when nothing matches.
fn fold_css(selectors: &[String]) -> String {
    if selectors.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<&str> = selectors.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    format!("{}{{display:none!important}}\n", sorted.join(","))
}

/// Site-specific hide rules for `url` from the cookie-list engine.
fn list_css_for_url(url: &str) -> String {
    let Some(lock) = ENGINE.get() else {
        return String::new();
    };
    let engine = lock.read().unwrap_or_else(|p| p.into_inner()).clone();
    let res = engine.url_cosmetic_resources(url);
    let selectors: Vec<String> = res.hide_selectors.into_iter().collect();
    fold_css(&selectors)
}

/// Generic hide rules matching the page's observed class/id names.
fn generic_css(url: &str, classes: &[String], ids: &[String]) -> String {
    let Some(lock) = ENGINE.get() else {
        return String::new();
    };
    let engine = lock.read().unwrap_or_else(|p| p.into_inner()).clone();
    let res = engine.url_cosmetic_resources(url);
    if res.generichide {
        return String::new();
    }
    let no_exceptions = HashSet::new();
    let selectors: Vec<String> = engine
        .hidden_class_id_selectors(classes, ids, &no_exceptions)
        .into_iter()
        .collect();
    fold_css(&selectors)
}

// ─── Toggle state (persisted, default on) ────────────────────────────────

/// Persisted cookie-consent state. A struct (not a bare bool) for forward-compat
/// (per-site exceptions, tier flags) without a migration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
struct ConsentState {
    enabled: bool,
}

impl Default for ConsentState {
    fn default() -> Self {
        // Reject-first is the product default (ADR-0026), like ad-blocking.
        Self { enabled: true }
    }
}

static STATE: Mutex<ConsentState> = Mutex::new(ConsentState { enabled: true });

/// Is cookie-consent handling (reject + hide) on?
pub fn is_enabled() -> bool {
    STATE.lock().unwrap_or_else(|p| p.into_inner()).enabled
}

/// Turn cookie-consent handling on/off (persisted). Takes effect on the next
/// page load (the host reloads the active tab — see [`crate::embed`]).
pub fn set_enabled(enabled: bool) {
    let changed = {
        let mut state = STATE.lock().unwrap_or_else(|p| p.into_inner());
        let changed = state.enabled != enabled;
        state.enabled = enabled;
        changed
    };
    if changed {
        save();
    }
}

/// Load persisted state into the global. Absent/corrupt ⇒ default (on). Call once
/// at startup (from [`crate::embed::initialize`]).
pub fn load_persisted() {
    if let Some(state) = load_from(&state_path()) {
        *STATE.lock().unwrap_or_else(|p| p.into_inner()) = state;
    }
}

fn state_path() -> String {
    format!("{}/consent.json", crate::root_cache_dir())
}

fn load_from(path: &str) -> Option<ConsentState> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(state) => Some(state),
        Err(err) => {
            eprintln!("[consent] failed to parse {path}: {err}");
            None
        }
    }
}

fn save() {
    let snapshot = *STATE.lock().unwrap_or_else(|p| p.into_inner());
    let path = state_path();
    let Ok(json) = serde_json::to_vec_pretty(&snapshot) else {
        return;
    };
    let tmp = format!("{path}.tmp");
    if let Err(err) = std::fs::write(&tmp, &json).and_then(|()| std::fs::rename(&tmp, &path)) {
        eprintln!("[consent] failed to write {path}: {err}");
    }
}

// ─── Substrate consumer ──────────────────────────────────────────────────

/// Register cookie-consent as a substrate consumer. Called once at startup
/// ([`crate::embed::initialize`]), before any browser exists.
pub fn register() {
    register_consumer(Consumer {
        channel: CHANNEL,
        script: OPERATOR_SCRIPT,
        handler: handle,
    });
}

/// Handle a cookie-consent message from the operator.
pub fn handle(msg: &Message) -> Option<serde_json::Value> {
    match msg.data.get("type").and_then(|v| v.as_str())? {
        "config" => Some(config_reply(is_enabled())),
        "cosmetic" => {
            if !is_enabled() {
                return Some(serde_json::json!({ "css": "" }));
            }
            let url = msg.data.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let classes = json_str_array(msg.data.get("classes"));
            let ids = json_str_array(msg.data.get("ids"));
            let site = list_css_for_url(url);
            let generic = generic_css(url, &classes, &ids);
            let css = match (site.is_empty(), generic.is_empty()) {
                (true, true) => String::new(),
                (false, true) => site,
                (true, false) => generic,
                (false, false) => format!("{site}{generic}"),
            };
            Some(serde_json::json!({ "css": css }))
        }
        "rejected" => {
            let cmp = msg.data.get("cmp").and_then(|v| v.as_str()).unwrap_or("?");
            let host = msg.data.get("host").and_then(|v| v.as_str()).unwrap_or("?");
            eprintln!("[consent] rejected {cmp} on {host}");
            note_rejected(msg.tab_id);
            Some(serde_json::json!({}))
        }
        _ => None,
    }
}

fn json_str_array(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The operator config: nothing when off; the reject ruleset + hide stylesheet
/// when on. Pure, so it is testable without touching the global toggle.
fn config_reply(enabled: bool) -> serde_json::Value {
    if !enabled {
        return serde_json::json!({ "enabled": false });
    }
    serde_json::json!({
        "enabled": true,
        "cmps": ruleset().get("cmps").cloned().unwrap_or_else(|| serde_json::json!([])),
        "hideCss": COOKIE_HIDE_CSS,
    })
}

/// The bundled reject ruleset. Each CMP: `present` (a selector proving the CMP is
/// on the page) + `reject` (candidate "reject all" controls, tried in order).
fn ruleset() -> serde_json::Value {
    serde_json::from_str(RULESET_JSON).expect("consent ruleset is valid JSON")
}

const RULESET_JSON: &str = r##"{
  "cmps": [
    {
      "name": "onetrust",
      "present": "#onetrust-banner-sdk, #onetrust-consent-sdk",
      "reject": ["#onetrust-reject-all-handler", "#onetrust-pc-btn-handler + #onetrust-pc-sdk #onetrust-reject-all-handler", ".ot-pc-refuse-all-handler", "button#onetrust-reject-btn-handler"]
    },
    {
      "name": "cookiebot",
      "present": "#CybotCookiebotDialog",
      "reject": ["#CybotCookiebotDialogBodyButtonDecline", "#CybotCookiebotDialogBodyLevelButtonLevelOptinDeclineAll"]
    },
    {
      "name": "didomi",
      "present": "#didomi-host, #didomi-notice",
      "reject": ["#didomi-notice-disagree-button", ".didomi-continue-without-agreeing", "button.didomi-components-button.didomi-button-highlight[disabled] ~ button"]
    },
    {
      "name": "quantcast",
      "present": ".qc-cmp2-container, .qc-cmp-ui-container",
      "reject": ["#disagree-btn", "#reject-all-btn", ".qc-cmp2-summary-buttons button[mode=\"secondary\"]", "button[aria-label=\"Disagree\"]"]
    },
    {
      "name": "sourcepoint",
      "present": "[id^=\"sp_message_container_\"], #sp-cc",
      "reject": ["button.sp_choice_type_11", "button.sp_choice_type_REJECT_ALL", "button[title=\"REJECT ALL\"]", "button[aria-label=\"Reject All\"]"]
    },
    {
      "name": "usercentrics",
      "present": "#usercentrics-root",
      "reject": ["button[data-testid=\"uc-deny-all-button\"]", "[data-testid=\"uc-deny-all-button\"]"]
    },
    {
      "name": "termly",
      "present": "#termly-code-snippet-support",
      "reject": ["button.t-declineAllButton", ".t-declineAllButton"]
    },
    {
      "name": "complianz",
      "present": "#cmplz-cookiebanner-container, .cmplz-cookiebanner",
      "reject": [".cmplz-btn.cmplz-deny", ".cmplz-deny"]
    },
    {
      "name": "iubenda",
      "present": "#iubenda-cs-banner",
      "reject": [".iub-btn-reject", "button.iubenda-cs-reject-btn", ".iubenda-cs-reject-btn"]
    },
    {
      "name": "cookieyes",
      "present": ".cky-consent-container, .cky-modal",
      "reject": [".cky-btn-reject", "[data-cky-tag=\"reject-button\"]"]
    },
    {
      "name": "osano",
      "present": ".osano-cm-window",
      "reject": [".osano-cm-denyAll", ".osano-cm-button--type_denyAll"]
    },
    {
      "name": "klaro",
      "present": ".klaro, #klaro",
      "reject": [".cm-btn.cm-btn-danger", ".cn-decline", ".cookie-modal .cm-btn-danger"]
    }
  ]
}"##;

/// Tier A — consent-plugin containers injected before the list engine's generic
/// pass runs. Low false-positive: plugin-specific IDs/classes only.
const COOKIE_HIDE_CSS: &str = "#onetrust-consent-sdk,\
#onetrust-banner-sdk,\
#CybotCookiebotDialog,\
#cookie-law-info-bar,\
#cookie-notice,\
#catapult-cookie-bar,\
#cmplz-cookiebanner-container,\
.cc-window.cc-banner,\
.osano-cm-window,\
#hs-eu-cookie-confirmation,\
#usercentrics-root,\
.fc-consent-root,\
#didomi-host,\
.qc-cmp2-container,\
.qc-cmp-ui-container,\
#truste-consent-track,\
#iubenda-cs-banner,\
#termly-code-snippet-support,\
#sp-cc,\
[id^=sp_message_container_],\
.cky-consent-container,\
.klaro .cookie-modal,\
#klaro,\
#pubtech-cmp,\
#cookiescript_injected,\
#cookiebanner,\
#cookie-banner,\
#cookieBanner,\
#cookie-consent,\
#cookieConsent,\
#gdpr-cookie-message,\
#moove_gdpr_cookie_info_bar{display:none!important}";

/// The operator (isolated world, document-start). Fetches its gated config; when
/// on, injects hide CSS (plugin + list + generic) and runs the Tier-B reject loop.
const OPERATOR_SCRIPT: &str = r##"(function () {
  if (!window.__browserCore || window.__cookieConsent) return;
  window.__cookieConsent = true;

  function injectCss(css, attr) {
    if (!css) return;
    var style = document.createElement('style');
    style.setAttribute(attr, '');
    style.textContent = css;
    (document.head || document.documentElement).appendChild(style);
  }

  function collectClassIds() {
    var classes = {}, ids = {};
    var nodes = document.querySelectorAll('[class],[id]');
    for (var i = 0; i < nodes.length; i++) {
      var el = nodes[i];
      if (el.id) ids[el.id] = 1;
      var cls = el.classList;
      if (cls) {
        for (var j = 0; j < cls.length; j++) classes[cls[j]] = 1;
      }
    }
    return { classes: Object.keys(classes), ids: Object.keys(ids) };
  }

  var injectedCosmetic = {};

  function fetchCosmetic() {
    var snap = collectClassIds();
    return window.__browserCore.send('consent', {
      type: 'cosmetic', url: location.href, classes: snap.classes, ids: snap.ids
    }).then(function (res) {
      var css = res && res.css;
      if (!css || injectedCosmetic[css]) return;
      injectedCosmetic[css] = true;
      injectCss(css, 'data-cookie-cosmetic');
    });
  }

  window.__browserCore.send('consent', { type: 'config' }).then(function (cfg) {
    if (!cfg || !cfg.enabled) return;

    injectCss(cfg.hideCss, 'data-cookie-hide');

    var cmps = cfg.cmps || [];
    var handled = {};

    function tryReject() {
      for (var i = 0; i < cmps.length; i++) {
        var cmp = cmps[i];
        if (handled[cmp.name] || !document.querySelector(cmp.present)) continue;
        for (var j = 0; j < cmp.reject.length; j++) {
          var btn = document.querySelector(cmp.reject[j]);
          if (btn) {
            btn.click();
            handled[cmp.name] = true;
            window.__browserCore.send('consent', {
              type: 'rejected', cmp: cmp.name, host: location.hostname
            });
            break;
          }
        }
      }
    }

    function tick() {
      tryReject();
      fetchCosmetic();
    }

    tick();
    var obs = new MutationObserver(tick);
    obs.observe(document.documentElement, {
      childList: true, subtree: true,
      attributes: true, attributeFilter: ['class', 'id']
    });
    setTimeout(function () { obs.disconnect(); }, 30000);
  });
})();"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_reply_is_gated_by_the_toggle() {
        let off = config_reply(false);
        assert_eq!(off, serde_json::json!({ "enabled": false }));
        assert!(off.get("cmps").is_none() && off.get("hideCss").is_none());

        let on = config_reply(true);
        assert_eq!(on["enabled"], true);
        assert!(on["cmps"].as_array().is_some_and(|a| !a.is_empty()));
        assert!(on["hideCss"]
            .as_str()
            .is_some_and(|s| s.contains("display:none")));
    }

    #[test]
    fn ruleset_lists_precise_cmp_rules() {
        let cmps = ruleset()["cmps"].as_array().expect("cmps array").clone();
        let names: Vec<&str> = cmps.iter().filter_map(|c| c["name"].as_str()).collect();
        assert!(names.contains(&"onetrust"));
        assert!(names.contains(&"cookiebot"));
        assert!(names.contains(&"quantcast"));
        assert!(names.contains(&"sourcepoint"));
        for cmp in &cmps {
            assert!(cmp["present"].as_str().is_some_and(|s| !s.is_empty()));
            assert!(!cmp["reject"].as_array().expect("reject array").is_empty());
        }
    }

    #[test]
    fn hide_css_covers_common_plugins() {
        assert!(COOKIE_HIDE_CSS.contains("#cookie-law-info-bar"));
        assert!(COOKIE_HIDE_CSS.contains("#onetrust-consent-sdk"));
        assert!(COOKIE_HIDE_CSS.contains("#cmplz-cookiebanner-container"));
        assert!(COOKIE_HIDE_CSS.ends_with("{display:none!important}"));
    }

    #[test]
    fn handle_config_returns_a_gated_reply_and_rejected_acks() {
        let cfg = Message {
            channel: CHANNEL,
            data: &serde_json::json!({ "type": "config" }),
            tab_id: 1,
        };
        assert_eq!(handle(&cfg).unwrap()["enabled"], true);

        let ack = Message {
            channel: CHANNEL,
            data: &serde_json::json!({ "type": "rejected", "cmp": "onetrust", "host": "x.com" }),
            tab_id: 1,
        };
        assert_eq!(handle(&ack), Some(serde_json::json!({})));

        let unknown = Message {
            channel: CHANNEL,
            data: &serde_json::json!({ "type": "frobnicate" }),
            tab_id: 1,
        };
        assert!(handle(&unknown).is_none());
    }

    #[test]
    fn cosmetic_reply_matches_generic_class_rules() {
        install_engine("##.cookie-alert\n###CookieBanner\n");
        let msg = Message {
            channel: CHANNEL,
            data: &serde_json::json!({
                "type": "cosmetic",
                "url": "https://news.example/",
                "classes": ["cookie-alert", "main"],
                "ids": []
            }),
            tab_id: 1,
        };
        let css = handle(&msg).unwrap()["css"].as_str().unwrap().to_string();
        assert!(css.contains(".cookie-alert"), "css: {css}");
    }

    #[test]
    fn rejected_message_increments_the_tabs_count() {
        let tab = 4242;
        reset_count(tab);
        assert_eq!(rejected_count(tab), 0);

        let ack = Message {
            channel: CHANNEL,
            data: &serde_json::json!({ "type": "rejected", "cmp": "onetrust", "host": "x.com" }),
            tab_id: tab,
        };
        handle(&ack);
        handle(&ack);
        assert_eq!(rejected_count(tab), 2);

        // A different tab's count is unaffected.
        assert_eq!(rejected_count(tab + 1), 0);

        reset_count(tab);
        assert_eq!(rejected_count(tab), 0);

        forget_tab(tab);
    }

    #[test]
    fn state_json_round_trips() {
        let s = ConsentState { enabled: false };
        let back: ConsentState = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(s, back);
        assert!(ConsentState::default().enabled);
    }

    #[test]
    fn operator_hides_rejects_and_fetches_cosmetics() {
        assert!(OPERATOR_SCRIPT.contains("type: 'config'"));
        assert!(OPERATOR_SCRIPT.contains("type: 'cosmetic'"));
        assert!(OPERATOR_SCRIPT.contains("cfg.hideCss"));
        assert!(OPERATOR_SCRIPT.contains("data-cookie-cosmetic"));
        assert!(OPERATOR_SCRIPT.contains("cmp.present"));
        assert!(OPERATOR_SCRIPT.contains(".click()"));
        assert!(OPERATOR_SCRIPT.contains("MutationObserver"));
    }
}
