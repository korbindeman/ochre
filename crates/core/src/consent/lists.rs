//! Cookie-annoyance list sourcing (P2-26, ADR-0026).
//!
//! Bundles a snapshot of Fanboy/EasyList Cookie cosmetic hide rules so Tier-A
//! coverage works offline from first launch, then background-refreshes from
//! upstream and hot-swaps the engine — same pattern as [`crate::adblock::lists`].

use std::time::Duration;

/// Compiled-in seed — cosmetic `##` / `###` rules extracted from Fanboy's
/// EasyList Cookie List. See `lists/seed.txt`.
pub const SEED: &str = include_str!("lists/seed.txt");

/// Fanboy/EasyList Cookie List — cosmetic hide rules only (no network rules).
const SOURCE: &str = "https://secure.fanboy.co.nz/fanboy-cookiemonster.txt";

const MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24 * 7); // 7 days
const DOWNLOAD_LIMIT: u64 = 8 * 1024 * 1024;

fn cache_path() -> String {
    format!("{}/consent-lists.txt", crate::root_cache_dir())
}

/// Keep only plain element-hiding cosmetic rules from the upstream list text.
/// Scriptlets, `:style()`, `:remove()`, and `:has()` need operators we don't
/// run yet — including them would bloat the engine without helping.
fn extract_cosmetic_rules(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('!') || line.starts_with('[') {
            continue;
        }
        // Domain-prefixed or global cosmetic hides.
        let rule = line.strip_prefix('*').unwrap_or(line);
        if !(rule.starts_with("##") || rule.starts_with("###")) {
            continue;
        }
        if rule.contains(":style(")
            || rule.contains(":remove()")
            || rule.contains(":has(")
            || rule.starts_with("##+js")
        {
            continue;
        }
        out.push_str(rule);
        out.push('\n');
    }
    out
}

pub fn load_cached() -> Option<String> {
    std::fs::read_to_string(cache_path())
        .ok()
        .filter(|s| !s.is_empty())
}

fn is_stale() -> bool {
    let Ok(meta) = std::fs::metadata(cache_path()) else {
        return true;
    };
    match meta.modified().ok().and_then(|m| m.elapsed().ok()) {
        Some(age) => age > MAX_AGE,
        None => true,
    }
}

fn fetch() -> Option<String> {
    let text = ureq::get(SOURCE)
        .call()
        .ok()?
        .body_mut()
        .with_config()
        .limit(DOWNLOAD_LIMIT)
        .lossy_utf8(true)
        .read_to_string()
        .ok()?;
    let rules = extract_cosmetic_rules(&text);
    (!rules.is_empty()).then_some(rules)
}

fn save_cache(text: &str) {
    let path = cache_path();
    let tmp = format!("{path}.tmp");
    if let Err(err) = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path)) {
        eprintln!("[consent] failed to write list cache {path}: {err}");
    }
}

/// Background upgrade: cached full list → upstream refresh when stale.
pub fn load_and_refresh() {
    let spawned = std::thread::Builder::new()
        .name("consent-lists".into())
        .spawn(|| {
            if let Some(cached) = load_cached() {
                super::install_engine(&cached);
            }
            if is_stale() {
                match fetch() {
                    Some(text) => {
                        save_cache(&text);
                        super::install_engine(&text);
                        eprintln!(
                            "[consent] cookie list refreshed ({} rules)",
                            text.lines().count()
                        );
                    }
                    None => {
                        eprintln!("[consent] cookie-list refresh failed; keeping current engine")
                    }
                }
            }
        });
    if let Err(err) = spawned {
        eprintln!("[consent] could not spawn list-refresh thread: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_has_cosmetic_rules() {
        assert!(SEED.contains("###CookieBanner"));
        assert!(SEED.contains("##.cookie-alert"));
        assert!(SEED.lines().count() > 1000);
    }

    #[test]
    fn extract_keeps_plain_hides_only() {
        let raw = "! comment\n||track^\n###CookieBanner\n##.cookie-alert\n##+js(trusted-click)\nexample.com##.x:has(.y)\n";
        let out = extract_cosmetic_rules(raw);
        assert!(out.contains("###CookieBanner"));
        assert!(out.contains("##.cookie-alert"));
        assert!(!out.contains("+js"));
        assert!(!out.contains(":has("));
        assert!(!out.contains("||track"));
    }
}
