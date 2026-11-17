//! The adblock-rust engine wrapper - domain resolver, engine construction, and
//! the block decision. Pure over the `adblock` crate: no global state and no
//! I/O (those live in [`super`] and [`super::lists`]), so it is unit-testable in
//! isolation.

use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;
use adblock::url_parser::ResolvesDomain;
use adblock::Engine;
use std::sync::Once;

/// A `psl`-backed domain resolver. adblock's `embedded-domain-resolver` feature
/// is disabled (spec §11), so we supply this - reusing the Public Suffix List
/// crate already compiled for the vault rather than adblock's separate `addr`.
struct PslResolver;

impl ResolvesDomain for PslResolver {
    fn get_host_domain(&self, host: &str) -> (usize, usize) {
        // The registrable domain (eTLD+1) is always a byte-suffix of `host`, so
        // its start index is `host_len - domain_len` - the exact computation the
        // crate's embedded resolver does. No valid domain ⇒ `(0, host.len())`,
        // per the trait contract.
        match psl::domain(host.as_bytes()) {
            Some(domain) => {
                let host_len = host.len();
                let domain_len = domain.as_bytes().len();
                (host_len.saturating_sub(domain_len), host_len)
            }
            None => (0, host.len()),
        }
    }
}

/// Register the domain resolver exactly once, process-wide. Must run before any
/// engine construction or request check - the crate panics on a check otherwise.
/// Idempotent; safe to call from every entry point.
pub fn ensure_resolver() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // The only error is a double-set, which `Once` already prevents.
        let _ = adblock::url_parser::set_domain_resolver(Box::new(PslResolver));
    });
}

/// Build an engine from newline-separated filter-list text (EasyList / uBO
/// syntax). `optimize` is on; debug info is off (we only read the block bit, and
/// debug strings cost memory).
pub fn build(rules: &str) -> Engine {
    ensure_resolver();
    let mut set = FilterSet::new(false);
    set.add_filter_list(rules, ParseOptions::default());
    Engine::from_filter_set(set, true)
}

/// Should this request be blocked by the network rules? `BlockerResult::matched`
/// already accounts for exception (allowlist) rules - an exception rule flips it
/// to `false` - so the block decision is exactly that bit.
///
/// `request_type` is the lowercase resource kind adblock expects (`"script"`,
/// `"image"`, `"stylesheet"`, `"sub_frame"`, `"xmlhttprequest"`, …); see
/// [`super::resource_type_name`] for the CEF mapping.
pub fn should_block(engine: &Engine, url: &str, source_url: &str, request_type: &str) -> bool {
    match Request::new(url, source_url, request_type) {
        Ok(request) => engine.check_network_request(&request).matched,
        // Non-network schemes (data:, blob:, about:, …) don't parse and are
        // never a blockable ad request - fail open.
        Err(_) => false,
    }
}

/// The cosmetic-filter stylesheet for `url` - the element-hiding selectors folded
/// into one `display:none!important` rule, ready to inject as a `<style>` at
/// document-start (P2-21, ADR-0020). Empty when the URL has no hide rules.
///
/// **Plain `hide_selectors` only.** The engine also yields `procedural_actions`
/// (JSON-encoded `:has()`/`:style()`/… filters needing a JS operator to apply),
/// generic class/id cosmetics (which need the page's observed class/id names via
/// a separate `hidden_class_id_selectors` round-trip), and scriptlets (which need
/// main-world execution) - all follow-ups. Selectors are sorted for stable output.
pub fn cosmetic_css(engine: &Engine, url: &str) -> String {
    let res = engine.url_cosmetic_resources(url);
    if res.hide_selectors.is_empty() {
        return String::new();
    }
    let mut selectors: Vec<&str> = res.hide_selectors.iter().map(String::as_str).collect();
    selectors.sort_unstable();
    format!("{}{{display:none!important}}\n", selectors.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny rule set exercising a network block, a third-party option, and an
    /// exception (allowlist) rule.
    fn test_engine() -> Engine {
        build(
            "||ads.example^\n\
             ||track.example^$third-party\n\
             @@||ads.example/allowed.js^\n",
        )
    }

    #[test]
    fn blocks_a_matching_host() {
        let engine = test_engine();
        assert!(should_block(
            &engine,
            "https://ads.example/banner.js",
            "https://news.example/",
            "script",
        ));
    }

    #[test]
    fn allows_an_unmatched_host() {
        let engine = test_engine();
        assert!(!should_block(
            &engine,
            "https://cdn.news.example/app.js",
            "https://news.example/",
            "script",
        ));
    }

    #[test]
    fn exception_rule_unblocks() {
        let engine = test_engine();
        // The `@@` exception for allowed.js beats the `||ads.example^` block.
        assert!(!should_block(
            &engine,
            "https://ads.example/allowed.js",
            "https://news.example/",
            "script",
        ));
    }

    #[test]
    fn third_party_option_respected() {
        let engine = test_engine();
        // Third-party (different registrable domain) ⇒ blocked…
        assert!(should_block(
            &engine,
            "https://track.example/p.gif",
            "https://news.example/",
            "image",
        ));
        // …first-party (same site) ⇒ the `$third-party` rule does not apply.
        assert!(!should_block(
            &engine,
            "https://track.example/p.gif",
            "https://track.example/",
            "image",
        ));
    }

    #[test]
    fn unparseable_url_fails_open() {
        let engine = test_engine();
        assert!(!should_block(
            &engine,
            "data:text/html,hi",
            "https://news.example/",
            "document",
        ));
    }

    #[test]
    fn cosmetic_css_hides_site_specific_selectors() {
        // `##` = generic hide; `example.com##` = site-specific. We build both and
        // confirm the site rule reaches example.com and not another host.
        let engine = build("example.com##.ad-banner\nexample.com###promo\n");
        let css = cosmetic_css(&engine, "https://example.com/page");
        assert!(css.contains(".ad-banner"), "css: {css}");
        assert!(css.contains("#promo"), "css: {css}");
        assert!(css.contains("display:none!important"));
        // A different site sees none of example.com's rules.
        assert!(cosmetic_css(&engine, "https://other.com/").is_empty());
    }

    #[test]
    fn psl_resolver_finds_registrable_domain() {
        ensure_resolver();
        let r = PslResolver;
        let host = "api.m.example.com";
        let (s, e) = r.get_host_domain(host);
        assert_eq!(&host[s..e], "example.com");
        let host = "a.b.co.uk";
        let (s, e) = r.get_host_domain(host);
        assert_eq!(&host[s..e], "b.co.uk");
    }
}
