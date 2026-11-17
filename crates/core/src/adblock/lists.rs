//! Filter-list sourcing: the compiled-in seed, the on-disk cache of the full
//! upstream lists, and the background task that upgrades the seed to the cached
//! lists and refreshes them from upstream - hot-swapping the engine each time.
//!
//! Startup builds from [`SEED`] synchronously (instant, offline). Everything
//! heavier - parsing the multi-MB cached lists, and the network fetch - runs on
//! a background thread via [`load_and_refresh`], so it never delays launch or
//! the IO thread.

use std::time::Duration;

/// Compiled-in seed list - effective on the first launch, offline, until the
/// full lists are fetched and hot-swapped in. See `lists/seed.txt`.
pub const SEED: &str = include_str!("lists/seed.txt");

/// Canonical upstream lists (uBO-compatible EasyList syntax).
const SOURCES: &[&str] = &[
    "https://easylist.to/easylist/easylist.txt",
    "https://easylist.to/easylist/easyprivacy.txt",
];

/// Refresh the cached lists in the background when they're older than this.
const MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24 * 3); // 3 days

/// Per-download cap. EasyList + EasyPrivacy are a few MB combined; this guards a
/// runaway response while sitting well above the real size.
const DOWNLOAD_LIMIT: u64 = 32 * 1024 * 1024;

fn cache_path() -> String {
    format!("{}/adblock-lists.txt", crate::root_cache_dir())
}

/// The cached combined list text from a prior fetch, if present. Returned
/// regardless of age - a stale full list still beats the seed; freshness only
/// decides whether to refresh (see [`load_and_refresh`]).
pub fn load_cached() -> Option<String> {
    std::fs::read_to_string(cache_path())
        .ok()
        .filter(|s| !s.is_empty())
}

fn is_stale() -> bool {
    let Ok(meta) = std::fs::metadata(cache_path()) else {
        return true; // no cache yet
    };
    match meta.modified().ok().and_then(|m| m.elapsed().ok()) {
        Some(age) => age > MAX_AGE,
        None => true,
    }
}

/// Fetch every source and concatenate. Returns `None` if any fetch fails - a
/// partial merge would silently weaken blocking, so we keep the current engine
/// and retry on the next launch.
fn fetch_all() -> Option<String> {
    let mut combined = String::new();
    for url in SOURCES {
        let text = ureq::get(*url)
            .call()
            .ok()?
            .body_mut()
            .with_config()
            .limit(DOWNLOAD_LIMIT)
            .lossy_utf8(true)
            .read_to_string()
            .ok()?;
        combined.push_str(&text);
        combined.push('\n');
    }
    Some(combined)
}

fn save_cache(text: &str) {
    let path = cache_path();
    let tmp = format!("{path}.tmp");
    if let Err(err) = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path)) {
        eprintln!("[adblock] failed to write list cache {path}: {err}");
    }
}

/// On a background thread: upgrade the seed engine to the cached full lists (if
/// any), then fetch fresh lists from upstream when the cache is missing or
/// stale, caching and hot-swapping the engine. A fetch failure is logged and
/// left for the next launch - the seed / cached engine keeps working.
pub fn load_and_refresh() {
    let spawned = std::thread::Builder::new()
        .name("adblock-lists".into())
        .spawn(|| {
            // 1. Seed → cached full lists, immediately, if we have them.
            if let Some(cached) = load_cached() {
                super::install_engine(&cached);
            }
            // 2. Refresh from upstream when missing or stale.
            if is_stale() {
                match fetch_all() {
                    Some(text) => {
                        save_cache(&text);
                        super::install_engine(&text);
                        eprintln!("[adblock] filter lists refreshed ({} bytes)", text.len());
                    }
                    None => {
                        eprintln!("[adblock] filter-list refresh failed; keeping current engine")
                    }
                }
            }
        });
    if let Err(err) = spawned {
        eprintln!("[adblock] could not spawn list-refresh thread: {err}");
    }
}
