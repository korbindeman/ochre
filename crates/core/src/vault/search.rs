//! `vault_search(query)` — rank the whole vault against a free-text query.
//!
//! [`lookup`](crate::vault::lookup) answers "what fits *this page*". This answers
//! the other question a user has: "where is the login I saved under some other
//! host?" — the entry an exact-host/domain match can never reach (a shared SSO
//! account, a renamed site, credentials filed under a bare app name).
//!
//! **One implementation, both surfaces** (ADR-0049): the URL-bar popover's search
//! field and the Passwords page's list call this. The frontend does not filter —
//! otherwise every new frontend would reinvent the semantics and drift.
//!
//! Ranking reuses the omnibox scorer ([`crate::fuzzy`]), so searching a vault
//! feels like searching history or a URL: a subsequence match, biased toward
//! word-boundary and consecutive hits. A login matches if the query matches its
//! **name**, its **username**, or any of its **URI hosts**, and scores as its best
//! field. Ties break by recency then vault order — the same chain [`lookup`] uses
//! (ADR-0048), so a login you actually use leads an otherwise-equal field.

use crate::fuzzy::fuzzy_match;
use crate::vault::lookup::host_from_uri;
use crate::vault::sync::{Login, Vault};
use crate::vault::usage::UsageMap;
use std::cmp::Reverse;

/// Logins matching `query`, best-first. An **empty query lists the whole vault**
/// in vault order — the Passwords page's unfiltered state. `usage` breaks score
/// ties (see [`crate::vault::usage::snapshot`]).
pub fn search_logins<'a>(vault: &'a Vault, query: &str, usage: &UsageMap) -> Vec<&'a Login> {
    let query = query.trim();
    if query.is_empty() {
        return vault.logins.iter().collect();
    }
    let mut scored: Vec<(&Login, i32)> = vault
        .logins
        .iter()
        .filter_map(|login| login_score(login, query).map(|score| (login, score)))
        .collect();
    // Best score first, then most-recently-used; the stable sort leaves vault
    // order deciding among logins that tie on both.
    scored.sort_by_key(|(login, score)| {
        (
            Reverse(*score),
            Reverse(usage.get(&login.id).copied().unwrap_or(0)),
        )
    });
    scored.into_iter().map(|(login, _)| login).collect()
}

/// The best score across a login's searchable fields — `None` when the query
/// matches none of them (so the login is not offered at all).
fn login_score(login: &Login, query: &str) -> Option<i32> {
    let name = fuzzy_match(query, &login.name).map(|m| m.score);
    let username = login
        .username
        .as_deref()
        .and_then(|u| fuzzy_match(query, u))
        .map(|m| m.score);
    let host = login
        .uris
        .iter()
        .filter_map(|u| host_from_uri(&u.uri))
        .filter_map(|h| fuzzy_match(query, &h).map(|m| m.score))
        .max();
    [name, username, host].into_iter().flatten().max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::sync::{Login, LoginUri, UriMatch, Vault};

    fn login(id: &str, name: &str, username: Option<&str>, uri: &str) -> Login {
        Login {
            id: id.to_string(),
            name: name.to_string(),
            username: username.map(str::to_string),
            password: None,
            uris: if uri.is_empty() {
                vec![]
            } else {
                vec![LoginUri {
                    uri: uri.to_string(),
                    match_type: UriMatch::Domain,
                }]
            },
            totp: None,
            notes: None,
            folder: None,
        }
    }

    fn ids(logins: Vec<&Login>) -> Vec<String> {
        logins.into_iter().map(|l| l.id.clone()).collect()
    }

    fn vault() -> Vault {
        Vault {
            logins: vec![
                login("gh", "GitHub", Some("octocat"), "https://github.com"),
                login(
                    "mail",
                    "Gmail",
                    Some("me@gmail.com"),
                    "https://mail.google.com",
                ),
                login(
                    "bank",
                    "My Bank",
                    Some("alice"),
                    "https://examplebank.co.uk",
                ),
            ],
        }
    }

    #[test]
    fn empty_query_lists_the_whole_vault_in_vault_order() {
        assert_eq!(
            ids(search_logins(&vault(), "", &UsageMap::new())),
            vec!["gh", "mail", "bank"]
        );
        // Whitespace is not a query.
        assert_eq!(
            ids(search_logins(&vault(), "   ", &UsageMap::new())).len(),
            3
        );
    }

    #[test]
    fn matches_name_username_or_host() {
        let v = vault();
        // By name.
        assert_eq!(
            ids(search_logins(&v, "github", &UsageMap::new())),
            vec!["gh"]
        );
        // By username — "octocat" appears in no name or host.
        assert_eq!(
            ids(search_logins(&v, "octocat", &UsageMap::new())),
            vec!["gh"]
        );
        // By URI host — "examplebank" is not in the name "My Bank".
        assert_eq!(
            ids(search_logins(&v, "examplebank", &UsageMap::new())),
            vec!["bank"]
        );
    }

    #[test]
    fn reaches_a_login_stored_under_another_host() {
        // The whole point of search (P2-42): on some unrelated page, find the
        // Gmail entry whose URI host would never match this site.
        assert_eq!(
            ids(search_logins(&vault(), "gmail", &UsageMap::new())),
            vec!["mail"]
        );
    }

    #[test]
    fn non_matching_query_returns_nothing() {
        assert!(search_logins(&vault(), "zzzznope", &UsageMap::new()).is_empty());
    }

    #[test]
    fn fuzzy_subsequence_matches_like_the_omnibox() {
        // "gh" is a subsequence of "GitHub" — the omnibox scorer's behavior,
        // inherited rather than reimplemented.
        assert!(ids(search_logins(&vault(), "gh", &UsageMap::new())).contains(&"gh".to_string()));
    }

    #[test]
    fn recency_breaks_score_ties() {
        // Two logins that score identically on name — only recency separates them.
        let v = Vault {
            logins: vec![
                login("a", "Shared", Some("a@x.com"), "https://a.example"),
                login("b", "Shared", Some("b@x.com"), "https://b.example"),
            ],
        };
        assert_eq!(
            ids(search_logins(&v, "shared", &UsageMap::new())),
            vec!["a", "b"],
            "no usage ⇒ vault order"
        );
        let usage = UsageMap::from([("b".to_string(), 500)]);
        assert_eq!(
            ids(search_logins(&v, "shared", &usage)),
            vec!["b", "a"],
            "the used one leads an otherwise-identical match"
        );
    }

    #[test]
    fn a_better_field_match_outranks_recency() {
        // Recency is only a tie-break: a strong name hit still beats a weak hit
        // on a recently-used login.
        let v = Vault {
            logins: vec![
                login("weak", "Zebra Warehouse", None, "https://zw.example"),
                login("strong", "GitHub", None, "https://github.com"),
            ],
        };
        let usage = UsageMap::from([("weak".to_string(), 999_999)]);
        assert_eq!(ids(search_logins(&v, "github", &usage))[0], "strong");
    }

    #[test]
    fn a_login_with_no_uris_is_still_searchable_by_name() {
        let v = Vault {
            logins: vec![login("app", "Some App", Some("user"), "")],
        };
        assert_eq!(
            ids(search_logins(&v, "some app", &UsageMap::new())),
            vec!["app"]
        );
    }
}
