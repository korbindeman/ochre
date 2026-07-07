//! `vault_lookup(host)` — match a page host to stored logins (ADR-0016, spec §9/§10).
//!
//! Given a page host, return the logins whose URIs match, **best-first** (an
//! exact-host match ranks above a registrable-domain match). Matching honors
//! Bitwarden's per-URI [`UriMatch`] rule:
//!
//! - **Domain** (the default) — same registrable domain via the Public Suffix
//!   List, so `google.com` creds match `accounts.google.com` but nothing leaks
//!   across a public suffix (`a.co.uk` never matches `b.co.uk`).
//! - **Host** — exact host equality.
//! - **Never** — never offered (a deliberate user override).
//!
//! The lookup input is a **host**, so the full-URL rules degrade safely: **Exact**
//! and **StartsWith** fall back to exact-host equality (a subset — we never
//! over-offer beyond the host), and **RegEx** is treated as no-match (no regex
//! engine yet; failing closed is the safe choice). Path-precise Exact/StartsWith
//! and RegEx arrive if/when lookup grows to take a full URL — filed, not silent.

use crate::vault::sync::{Login, LoginUri, UriMatch, Vault};

/// Logins matching `host`, strongest match first.
pub fn matching_logins<'a>(vault: &'a Vault, host: &str) -> Vec<&'a Login> {
    let host = normalize_host(host);
    if host.is_empty() {
        return Vec::new();
    }
    let host_domain = registrable_domain(&host);

    let mut scored: Vec<(&Login, MatchQuality)> = vault
        .logins
        .iter()
        .filter_map(|login| login_quality(login, &host, host_domain.as_deref()).map(|q| (login, q)))
        .collect();
    // Ord on MatchQuality puts Host before Domain; stable sort keeps vault order
    // within a tier.
    scored.sort_by_key(|(_, quality)| *quality);
    scored.into_iter().map(|(login, _)| login).collect()
}

/// How strongly a login matched — `Host` beats `Domain`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MatchQuality {
    Host,
    Domain,
}

/// The strongest match across a login's URIs, if any.
fn login_quality(login: &Login, host: &str, host_domain: Option<&str>) -> Option<MatchQuality> {
    login
        .uris
        .iter()
        .filter_map(|uri| uri_quality(uri, host, host_domain))
        .min()
}

fn uri_quality(
    login_uri: &LoginUri,
    host: &str,
    host_domain: Option<&str>,
) -> Option<MatchQuality> {
    // RegEx: no engine yet — fail closed. Never: explicit user opt-out.
    if matches!(login_uri.match_type, UriMatch::Never | UriMatch::RegEx) {
        return None;
    }
    let uri_host = host_from_uri(&login_uri.uri)?;

    if uri_host == host {
        return Some(MatchQuality::Host);
    }
    match login_uri.match_type {
        // Host/Exact/StartsWith need an exact host (their full-URL fidelity needs
        // a full-URL input, not just a host) — already handled by the check above.
        UriMatch::Host | UriMatch::Exact | UriMatch::StartsWith => None,
        // Domain: same registrable domain counts.
        UriMatch::Domain => match (registrable_domain(&uri_host), host_domain) {
            (Some(a), Some(b)) if a == b => Some(MatchQuality::Domain),
            _ => None,
        },
        UriMatch::Never | UriMatch::RegEx => None,
    }
}

/// Lower-case, trim, drop a trailing root dot.
fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_lowercase()
}

/// The registrable domain (eTLD+1) of a host, via the Public Suffix List.
fn registrable_domain(host: &str) -> Option<String> {
    psl::domain_str(host).map(|d| d.to_lowercase())
}

/// Extract the host from a stored URI, which may be a full URL
/// (`https://user@example.com:8443/login?x`) or a bare host (`example.com`).
/// IPv6 literals are not handled (vanishingly rare for login URIs). Shared with
/// the importer (deriving a display name from a URL).
pub(crate) fn host_from_uri(uri: &str) -> Option<String> {
    let s = uri.trim();
    // Drop the scheme.
    let s = match s.find("://") {
        Some(i) => &s[i + 3..],
        None => s,
    };
    // The authority ends at the first path/query/fragment delimiter.
    let end = s.find(['/', '?', '#']).unwrap_or(s.len());
    let authority = &s[..end];
    // Drop any userinfo.
    let authority = match authority.rfind('@') {
        Some(i) => &authority[i + 1..],
        None => authority,
    };
    // Drop a port.
    let host = match authority.rfind(':') {
        Some(i) => &authority[..i],
        None => authority,
    };
    let host = normalize_host(host);
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::sync::{Login, LoginUri, UriMatch, Vault};

    fn login(id: &str, uris: &[(&str, UriMatch)]) -> Login {
        Login {
            id: id.to_string(),
            name: id.to_string(),
            username: None,
            password: None,
            uris: uris
                .iter()
                .map(|(u, m)| LoginUri {
                    uri: u.to_string(),
                    match_type: *m,
                })
                .collect(),
            totp: None,
            notes: None,
            folder: None,
        }
    }

    fn ids(logins: Vec<&Login>) -> Vec<String> {
        logins.into_iter().map(|l| l.id.clone()).collect()
    }

    #[test]
    fn host_from_uri_extracts_authority() {
        assert_eq!(
            host_from_uri("https://example.com/login").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            host_from_uri("http://user:pw@Example.COM:8443/x?y").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_from_uri("example.com").as_deref(), Some("example.com"));
        assert_eq!(
            host_from_uri("https://sub.example.co.uk").as_deref(),
            Some("sub.example.co.uk")
        );
        assert_eq!(host_from_uri(""), None);
    }

    #[test]
    fn domain_match_spans_subdomains() {
        let vault = Vault {
            logins: vec![login("g", &[("https://google.com", UriMatch::Domain)])],
        };
        // Same registrable domain, different subdomain → matches.
        assert_eq!(
            ids(matching_logins(&vault, "accounts.google.com")),
            vec!["g"]
        );
        assert_eq!(ids(matching_logins(&vault, "google.com")), vec!["g"]);
    }

    #[test]
    fn domain_match_never_leaks_across_a_public_suffix() {
        // The footgun a naive "last two labels" matcher would hit: a.co.uk must
        // NOT match b.co.uk (registrable domains differ: a.co.uk vs b.co.uk).
        let vault = Vault {
            logins: vec![login("a", &[("https://a.co.uk", UriMatch::Domain)])],
        };
        assert!(matching_logins(&vault, "b.co.uk").is_empty());
        assert_eq!(ids(matching_logins(&vault, "www.a.co.uk")), vec!["a"]);
    }

    #[test]
    fn host_match_requires_exact_host() {
        let vault = Vault {
            logins: vec![login(
                "h",
                &[("https://accounts.google.com", UriMatch::Host)],
            )],
        };
        assert_eq!(
            ids(matching_logins(&vault, "accounts.google.com")),
            vec!["h"]
        );
        // A sibling subdomain does NOT match under Host.
        assert!(matching_logins(&vault, "mail.google.com").is_empty());
        // The bare domain does not match a host-scoped subdomain URI.
        assert!(matching_logins(&vault, "google.com").is_empty());
    }

    #[test]
    fn never_rule_excludes_and_regex_fails_closed() {
        let vault = Vault {
            logins: vec![
                login("never", &[("https://example.com", UriMatch::Never)]),
                login("regex", &[("https://example.com", UriMatch::RegEx)]),
            ],
        };
        assert!(matching_logins(&vault, "example.com").is_empty());
    }

    #[test]
    fn exact_and_startswith_degrade_to_host_equality() {
        let vault = Vault {
            logins: vec![
                login("e", &[("https://example.com/login", UriMatch::Exact)]),
                login("s", &[("https://shop.example.com", UriMatch::StartsWith)]),
            ],
        };
        // Same host → offered (a safe subset of the true full-URL rule).
        assert_eq!(ids(matching_logins(&vault, "example.com")), vec!["e"]);
        assert_eq!(ids(matching_logins(&vault, "shop.example.com")), vec!["s"]);
        // Different host under the same domain → NOT offered (Exact/StartsWith
        // do not widen to the domain).
        assert!(!matching_logins(&vault, "other.example.com")
            .iter()
            .any(|l| l.id == "e"));
    }

    #[test]
    fn exact_host_ranks_above_domain_match() {
        let vault = Vault {
            logins: vec![
                login("domain", &[("https://google.com", UriMatch::Domain)]),
                login("host", &[("https://accounts.google.com", UriMatch::Host)]),
            ],
        };
        // Querying the exact host: the Host match must come first.
        assert_eq!(
            ids(matching_logins(&vault, "accounts.google.com")),
            vec!["host", "domain"]
        );
    }

    #[test]
    fn no_match_and_empty_host_return_nothing() {
        let vault = Vault {
            logins: vec![login("g", &[("https://google.com", UriMatch::Domain)])],
        };
        assert!(matching_logins(&vault, "example.org").is_empty());
        assert!(matching_logins(&vault, "").is_empty());
        assert!(matching_logins(&vault, "   ").is_empty());
    }
}
