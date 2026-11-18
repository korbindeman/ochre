//! Ad blocking on WebKit (ADR-0072): the adblock-rust filter lists compiled to
//! WebKit content-rule JSON (the Brave iOS path), plus a page-world scriptlet
//! that strips YouTube's ad payloads.
//!
//! WebKit applies content rules inside its own network process, so there is no
//! per-request hook: the per-site allowlist ("turn off for this site") is a
//! trailing `ignore-previous-rules` rule with `if-domain`, which only affects
//! rules in the same list. Each allowlist change therefore recompiles the list
//! (a few seconds, off the main thread inside WebKit). Private windows add
//! their own ephemeral allowlist, so each profile compiles its own list.
//!
//! Site-specific element hiding still rides the injection substrate
//! ([`crate::adblock::cosmetic`]); the compiled rules add generic hiding.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use adblock::lists::{FilterSet, ParseOptions};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::{
    WKContentRuleList, WKContentRuleListStore, WKContentWorld, WKUserContentController,
    WKUserScript, WKUserScriptInjectionTime,
};

use super::Profile;

/// The compiled-rule JSON array for the current lists, without the allowlist.
static BASE_RULES: Mutex<Option<Arc<String>>> = Mutex::new(None);

thread_local! {
    /// The rule list currently attached to each profile's content controller.
    static ATTACHED: RefCell<HashMap<Profile, Retained<WKContentRuleList>>> = RefCell::new(HashMap::new());
    /// Bumped per compile so a slow, stale compile never replaces a newer one.
    static GENERATION: RefCell<HashMap<Profile, u64>> = RefCell::new(HashMap::new());
}

/// Convert filter-list text to WebKit content-rule JSON. Runs on the list
/// thread (conversion of the full lists takes a fraction of a second).
pub fn convert(list_text: &str) -> String {
    let mut set = FilterSet::new(true);
    set.add_filter_list(list_text, ParseOptions::default());
    match set.into_content_blocking() {
        Ok((rules, _)) => serde_json::to_string(&rules).unwrap_or_else(|_| "[]".into()),
        Err(()) => "[]".into(),
    }
}

/// New lists arrived (startup seed, cache, or refresh): store their rules and
/// recompile every profile's list on the main thread.
pub fn set_lists(list_text: &str) {
    let json = Arc::new(convert(list_text));
    *BASE_RULES.lock().unwrap_or_else(|p| p.into_inner()) = Some(json);
    crate::main_thread::post(refresh);
}

/// `base` plus one trailing rule that turns blocking off on `allowlisted` sites.
pub(crate) fn with_allowlist(base: &str, allowlisted: &[String]) -> String {
    let domains: Vec<String> = allowlisted
        .iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty() && h.is_ascii())
        .map(|h| format!("*{h}"))
        .collect();
    if domains.is_empty() {
        return base.to_string();
    }
    let rule = serde_json::json!({
        "trigger": { "url-filter": ".*", "if-domain": domains },
        "action": { "type": "ignore-previous-rules" }
    });
    let trimmed = base.trim_end();
    let Some(body) = trimmed.strip_suffix(']') else {
        return base.to_string();
    };
    if body.trim_end().ends_with('[') {
        format!("[{rule}]")
    } else {
        format!("{body},{rule}]")
    }
}

fn allowlist_for(profile: Profile) -> Vec<String> {
    let mut hosts = crate::adblock::allowlisted_sites();
    if let Profile::Private(window_id) = profile {
        hosts.extend(crate::private_window::ephemeral_allowlist(window_id));
    }
    hosts
}

fn identifier(profile: Profile) -> String {
    match profile {
        Profile::Normal => "ochre-adblock".to_string(),
        Profile::Private(id) => format!("ochre-adblock-private-{id}"),
    }
}

/// A new profile's content controller: add the scripts, attach the list
/// compiled on the last run (WebKit keeps it on disk) so the first page of a
/// session is already filtered, then recompile against the current lists.
pub(super) fn attach(content: &WKUserContentController, profile: Profile) {
    install_scripts(content, profile);
    attach_previous(content, profile);
    let content = content.retain();
    crate::main_thread::post_local(move || compile(profile, &content));
}

/// Attach the normal profile's last compiled list, unless a fresh compile got
/// there first. Private windows start with the same list: their ephemeral
/// allowlist is empty when they open.
fn attach_previous(content: &WKUserContentController, profile: Profile) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if !crate::adblock::is_enabled() {
        return;
    }
    let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(mtm) }) else {
        return;
    };
    let content = content.retain();
    let found = RcBlock::new(move |list: *mut WKContentRuleList, _err: *mut NSError| {
        let Some(list) = (unsafe { list.as_ref() }) else {
            return;
        };
        if ATTACHED.with(|a| a.borrow().contains_key(&profile)) {
            return;
        }
        // SAFETY: main thread (WebKit calls back on the main queue).
        unsafe { content.addContentRuleList(list) };
        ATTACHED.with(|a| a.borrow_mut().insert(profile, list.retain()));
        eprintln!("[adblock] previous content rules attached for {profile:?}");
    });
    // SAFETY: main thread; asynchronous lookup.
    unsafe {
        store.lookUpContentRuleListForIdentifier_completionHandler(
            Some(&NSString::from_str(&identifier(Profile::Normal))),
            Some(&found),
        );
    }
}

/// Re-apply blocking to every profile: after new lists, the global switch,
/// or an allowlist change. Main thread.
pub fn refresh() {
    let mut targets = Vec::new();
    super::for_each_profile(|profile, content| targets.push((profile, content.retain())));
    for (profile, content) in targets {
        install_scripts(&content, profile);
        compile(profile, &content);
    }
}

fn detach(profile: Profile, content: &WKUserContentController) {
    if let Some(old) = ATTACHED.with(|a| a.borrow_mut().remove(&profile)) {
        // SAFETY: main thread; removing a list we added.
        unsafe { content.removeContentRuleList(&old) };
    }
}

fn compile(profile: Profile, content: &WKUserContentController) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if !crate::adblock::is_enabled() {
        detach(profile, content);
        return;
    }
    let Some(base) = BASE_RULES.lock().unwrap_or_else(|p| p.into_inner()).clone() else {
        return;
    };
    let json = with_allowlist(&base, &allowlist_for(profile));
    let generation = GENERATION.with(|g| {
        let mut g = g.borrow_mut();
        let n = g.entry(profile).or_insert(0);
        *n += 1;
        *n
    });
    let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(mtm) }) else {
        return;
    };
    let content = content.retain();
    let started = std::time::Instant::now();
    let done = RcBlock::new(move |list: *mut WKContentRuleList, err: *mut NSError| {
        let current = GENERATION.with(|g| g.borrow().get(&profile).copied()) == Some(generation);
        if !current {
            return;
        }
        match unsafe { list.as_ref() } {
            Some(list) => {
                detach(profile, &content);
                // SAFETY: main thread (WebKit calls back on the main queue).
                unsafe { content.addContentRuleList(list) };
                ATTACHED.with(|a| a.borrow_mut().insert(profile, list.retain()));
                eprintln!(
                    "[adblock] content rules ready for {profile:?} in {} ms",
                    started.elapsed().as_millis()
                );
            }
            None => eprintln!(
                "[adblock] content rules failed to compile: {:?}",
                unsafe { err.as_ref() }.map(|e| e.localizedDescription().to_string())
            ),
        }
    });
    // SAFETY: main thread; WebKit compiles on its own queue and calls back on main.
    unsafe {
        store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(&NSString::from_str(&identifier(profile))),
            Some(&NSString::from_str(&json)),
            Some(&done),
        );
    }
}

/// (Re)install the profile's user scripts: the injection substrate's scripts,
/// plus the YouTube scriptlet while blocking is on.
fn install_scripts(content: &WKUserContentController, profile: Profile) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: main thread; replacing this controller's scripts wholesale.
    unsafe { content.removeAllUserScripts() };
    crate::inject::add_scripts(content, mtm);
    let youtube_on =
        crate::adblock::is_enabled() && !allowlist_for(profile).iter().any(|h| h == "youtube.com");
    if youtube_on {
        // SAFETY: main thread; page world, document start, all frames.
        unsafe {
            let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                mtm.alloc(),
                &NSString::from_str(YOUTUBE_PRUNE_JS),
                WKUserScriptInjectionTime::AtDocumentStart,
                false,
                &WKContentWorld::pageWorld(mtm),
            );
            content.addUserScript(&script);
        }
    }
}

/// Page-world scriptlet (uBlock Origin's `json-prune` approach): YouTube puts
/// its ads in the player response, served from the same hosts as the video, so
/// no request rule can block them. Removing the ad fields before the player
/// reads them stops pre-rolls and mid-rolls. Runs only on YouTube hosts.
const YOUTUBE_PRUNE_JS: &str = r#"(function () {
  var h = location.hostname;
  if (!(h === 'youtube.com' || h.endsWith('.youtube.com') || h === 'youtube-nocookie.com' || h.endsWith('.youtube-nocookie.com'))) return;
  var keys = ['adPlacements', 'playerAds', 'adSlots', 'adBreakHeartbeatParams'];
  var prune = function (o) {
    if (o && typeof o === 'object') {
      for (var i = 0; i < keys.length; i++) if (keys[i] in o) delete o[keys[i]];
      if (o.playerResponse && typeof o.playerResponse === 'object') prune(o.playerResponse);
    }
    return o;
  };
  var value;
  try {
    Object.defineProperty(window, 'ytInitialPlayerResponse', {
      configurable: true,
      get: function () { return value; },
      set: function (v) { value = prune(v); }
    });
  } catch (e) {}
  var parse = JSON.parse;
  JSON.parse = function () { return prune(parse.apply(this, arguments)); };
  var json = Response.prototype.json;
  Response.prototype.json = function () { return json.call(this).then(prune); };
})();"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_a_network_rule_and_a_hiding_rule() {
        let json = convert("||ads.example^\nexample.com##.banner\n");
        let rules: serde_json::Value = serde_json::from_str(&json).unwrap();
        let text = rules.to_string();
        assert!(text.contains("block"), "{text}");
        assert!(text.contains("css-display-none"), "{text}");
    }

    #[test]
    fn allowlist_rule_is_appended_last() {
        let out = with_allowlist(r#"[{"a":1}]"#, &["Example.com".into()]);
        let rules: serde_json::Value = serde_json::from_str(&out).unwrap();
        let last = rules.as_array().unwrap().last().unwrap();
        assert_eq!(last["action"]["type"], "ignore-previous-rules");
        assert_eq!(last["trigger"]["if-domain"][0], "*example.com");
        assert_eq!(rules.as_array().unwrap().len(), 2);
    }

    #[test]
    fn allowlist_on_empty_and_no_allowlist_cases() {
        let out = with_allowlist("[]", &["a.com".into()]);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&out)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(with_allowlist("[1]", &[]), "[1]");
    }
}
