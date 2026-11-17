//! `browser_vault_search` across the real FFI boundary (P2-42, ADR-0049).
//!
//! The unit tests in `vault::search` prove the ranking; this proves the layer the
//! native popover actually consumes - the C ABI and the JSON it returns - against
//! the process-global vault, which a unit test inside `core` cannot do without
//! clobbering the shared test process's `VAULT` for every other test.
//!
//! Hermetic: `HOME` is repointed at a temp dir before anything touches
//! `root_cache_dir()`, so the usage store starts empty and the developer's real
//! vault is never read or written.

use browser_ffi::browser_vault_search;
use std::ffi::{CStr, CString};

/// Decoded `LoginView` fields the popover reads.
fn search(query: &str) -> serde_json::Value {
    let q = CString::new(query).unwrap();
    let raw = unsafe { browser_vault_search(q.as_ptr()) };
    assert!(!raw.is_null(), "search returned null for {query:?}");
    let json = unsafe { CStr::from_ptr(raw) }.to_str().unwrap().to_string();
    unsafe { browser_ffi::browser_string_free(raw) };
    serde_json::from_str(&json).expect("search returned valid JSON")
}

fn ids(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .expect("a JSON array")
        .iter()
        .map(|l| l["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn search_reaches_the_whole_vault_over_the_ffi() {
    let home = std::env::temp_dir().join("browser-ffi-vault-search-home");
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);

    // A locked / never-initialized vault must not panic across the boundary -
    // it yields an empty array, which the popover renders as "no results".
    assert_eq!(ids(&search("anything")), Vec::<String>::new());

    // The demo vault: "Demo Login" (octocat@example.com) + "Work Account"
    // (work@example.com), both on localhost.
    browser_core::vault::service::seed_demo_vault();

    // Empty query ⇒ the whole vault (the Passwords page's unfiltered list).
    assert_eq!(ids(&search("")), vec!["demo", "demo-work"]);

    // By name: "work" hits "Work Account", not "Demo Login".
    assert_eq!(ids(&search("work")), vec!["demo-work"]);

    // By username: "octocat" appears only in demo's username - the field the
    // host-based lookup could never reach.
    assert_eq!(ids(&search("octocat")), vec!["demo"]);

    // No match ⇒ empty, not everything.
    assert_eq!(ids(&search("zzzznope")), Vec::<String>::new());

    // The boundary's standing guarantee (ADR-0017): metadata only, never a
    // secret - `seed_demo_vault`'s passwords must not appear in the payload.
    let json = search("").to_string();
    assert!(!json.contains("password"), "password key leaked: {json}");
    assert!(
        !json.contains("correct horse battery staple") && !json.contains("hunter2-work-pw"),
        "a password value leaked into search JSON: {json}"
    );

    std::fs::remove_dir_all(&home).ok();
}
