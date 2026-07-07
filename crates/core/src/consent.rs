//! Cookie-consent — reject-first CMP automation + banner hiding (P2-22/P2-25, ADR-0026).
//!
//! EU consent banners (GDPR/ePrivacy CMPs) are handled by *opting out*, not just
//! hiding — hiding-without-rejecting leaves the user tracked behind a clean face,
//! which is the anti-pattern we position against (ADR-0026). This [`crate::inject`]
//! consumer runs both tiers, gated on one **cookie-consent toggle** (distinct from
//! ad-block):
//!
//! - **Tier B (reject)** — detect a known CMP and click its "reject all" control,
//!   so the CMP dismisses itself with consent *denied* (OneTrust/Cookiebot/Didomi
//!   via a core-served ruleset; a `MutationObserver` catches async banners).
//! - **Tier A (hide)** — a curated **consent-plugin stylesheet** injected at
//!   document-start hides the long-tail banners we have no reject rule for. The
//!   honest fallback: hidden ≠ rejected (the banner's gone but consent is
//!   unanswered). Hide and reject compose — a programmatic `.click()` fires even on
//!   a hidden button, so a CMP we both hide *and* reject is cleaned instantly *and*
//!   opted out underneath.
//!
//! Both tiers are served in one gated `config` message; when the toggle is off the
//! reply is `{enabled:false}` and the operator does nothing.
//!
//! ## Scope (first cut)
//!
//! Tier A is a **hand-authored, conservative** set of consent-plugin selectors —
//! not the full EasyList Cookie / uBO Annoyances lists (those are mostly *generic*
//! cosmetic rules, which need the deferred generic-cosmetic matching path, not the
//! site-specific `url_cosmetic_resources` P2-21 exposes). Broadening to the full
//! lists (+ hot-update, + cross-origin iframe CMPs via P2-24/ADR-0027) is P2-26.

use crate::inject::{register as register_consumer, Consumer, Message};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// The substrate channel cookie-consent messages ride.
const CHANNEL: &str = "consent";

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
        // The operator asks for its config on load (gated on the toggle).
        "config" => Some(config_reply(is_enabled())),
        // A reject fired — log it (observability) and ack so the promise resolves.
        "rejected" => {
            let cmp = msg.data.get("cmp").and_then(|v| v.as_str()).unwrap_or("?");
            let host = msg.data.get("host").and_then(|v| v.as_str()).unwrap_or("?");
            eprintln!("[consent] rejected {cmp} on {host}");
            Some(serde_json::json!({}))
        }
        _ => None,
    }
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
      "present": "#onetrust-banner-sdk",
      "reject": ["#onetrust-reject-all-handler", ".ot-pc-refuse-all-handler"]
    },
    {
      "name": "cookiebot",
      "present": "#CybotCookiebotDialog",
      "reject": ["#CybotCookiebotDialogBodyButtonDecline", "#CybotCookiebotDialogBodyLevelButtonLevelOptinDeclineAll"]
    },
    {
      "name": "didomi",
      "present": "#didomi-host",
      "reject": ["#didomi-notice-disagree-button", ".didomi-continue-without-agreeing"]
    }
  ]
}"##;

/// Tier A — a curated, conservative set of **consent-plugin-specific** selectors
/// (IDs/classes tied to a known consent plugin, so they match nothing on a page
/// without it — low false-positive). Injected as one `display:none` rule to hide
/// the long-tail banners we have no reject rule for. Not the full EasyList Cookie
/// list (that needs generic cosmetic matching — P2-26).
const COOKIE_HIDE_CSS: &str = "#onetrust-consent-sdk,\
#CybotCookiebotDialog,\
#cookie-law-info-bar,\
#cookie-notice,\
#catapult-cookie-bar,\
.cc-window.cc-banner,\
.osano-cm-window,\
#hs-eu-cookie-confirmation,\
#usercentrics-root,\
.fc-consent-root,\
#didomi-host,\
.qc-cmp2-container,\
#truste-consent-track,\
[id^=sp_message_container_]{display:none!important}";

/// The operator (isolated world, document-start). Fetches its gated config; when
/// on, injects the Tier-A hide stylesheet and runs the Tier-B reject loop (finds a
/// present CMP and clicks its reject control, re-trying on DOM mutations for
/// async-injected banners). Precise reject selectors only; each CMP rejected once.
const OPERATOR_SCRIPT: &str = r##"(function () {
  if (!window.__browserCore || window.__cookieConsent) return;
  window.__cookieConsent = true;

  window.__browserCore.send('consent', { type: 'config' }).then(function (cfg) {
    if (!cfg || !cfg.enabled) return;

    // Tier A — hide the long-tail banners.
    if (cfg.hideCss) {
      var style = document.createElement('style');
      style.setAttribute('data-cookie-hide', '');
      style.textContent = cfg.hideCss;
      (document.head || document.documentElement).appendChild(style);
    }

    // Tier B — reject known CMPs. `.click()` fires even on a button our Tier-A
    // CSS has hidden, so hide and reject compose. Each CMP is rejected once.
    var cmps = cfg.cmps || [];
    if (!cmps.length) return;
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

    tryReject();
    var obs = new MutationObserver(tryReject);
    obs.observe(document.documentElement, { childList: true, subtree: true });
    setTimeout(function () { obs.disconnect(); }, 10000);
  });
})();"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_reply_is_gated_by_the_toggle() {
        // Off ⇒ nothing but the flag (operator does nothing).
        let off = config_reply(false);
        assert_eq!(off, serde_json::json!({ "enabled": false }));
        assert!(off.get("cmps").is_none() && off.get("hideCss").is_none());

        // On ⇒ the reject ruleset + the hide stylesheet.
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
        assert!(names.contains(&"didomi"));
        for cmp in &cmps {
            assert!(cmp["present"].as_str().is_some_and(|s| !s.is_empty()));
            assert!(!cmp["reject"].as_array().expect("reject array").is_empty());
        }
    }

    #[test]
    fn hide_css_is_plugin_specific_and_hides() {
        // Consent-plugin selectors → display:none. Includes a couple used by the
        // live smoke.
        assert!(COOKIE_HIDE_CSS.contains("#cookie-law-info-bar"));
        assert!(COOKIE_HIDE_CSS.contains("#onetrust-consent-sdk"));
        assert!(COOKIE_HIDE_CSS.ends_with("{display:none!important}"));
        // Conservative: no bare generic class like ".cookie" that would over-hide.
        assert!(!COOKIE_HIDE_CSS.contains(",.cookie{") && !COOKIE_HIDE_CSS.contains(",.banner"));
    }

    #[test]
    fn handle_config_returns_a_gated_reply_and_rejected_acks() {
        // Default state in the test process is enabled.
        let cfg = Message {
            channel: CHANNEL,
            data: &serde_json::json!({ "type": "config" }),
        };
        assert_eq!(handle(&cfg).unwrap()["enabled"], true);

        let ack = Message {
            channel: CHANNEL,
            data: &serde_json::json!({ "type": "rejected", "cmp": "onetrust", "host": "x.com" }),
        };
        assert_eq!(handle(&ack), Some(serde_json::json!({})));

        let unknown = Message {
            channel: CHANNEL,
            data: &serde_json::json!({ "type": "frobnicate" }),
        };
        assert!(handle(&unknown).is_none());
    }

    #[test]
    fn state_json_round_trips() {
        let s = ConsentState { enabled: false };
        let back: ConsentState = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(s, back);
        assert!(ConsentState::default().enabled);
    }

    #[test]
    fn operator_is_gated_hides_and_rejects() {
        assert!(OPERATOR_SCRIPT.contains("type: 'config'"));
        assert!(OPERATOR_SCRIPT.contains("cfg.enabled"));
        assert!(OPERATOR_SCRIPT.contains("cfg.hideCss"));
        assert!(OPERATOR_SCRIPT.contains("createElement('style')"));
        assert!(OPERATOR_SCRIPT.contains("cmp.present"));
        assert!(OPERATOR_SCRIPT.contains(".click()"));
        assert!(OPERATOR_SCRIPT.contains("MutationObserver"));
    }
}
