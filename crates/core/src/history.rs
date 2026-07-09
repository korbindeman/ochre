//! Browsing-history + favicon store backing the command-bar autocomplete.
//!
//! A single SQLite database (rusqlite, bundled) at
//! `root_cache_dir()/history.sqlite` records every visited page (URL, title,
//! host, visit count, last-visit time) and caches one favicon PNG per host. The
//! command bar queries it for frecency-ranked suggestions as the user types.
//!
//! Threading: on macOS every write happens on CEF's UI thread (= the main
//! thread) from the display handler, and every read happens on that same thread
//! from the FFI query call. A single process-wide `Mutex<History>` therefore
//! sees no cross-thread contention — the lock is defensive, not a hot path.
//!
//! The store sits behind this small surface (`record_visit`, `update_title`,
//! `store_favicon`, `query`, `list`, `delete`, `clear_since`, `favicon`)
//! precisely so the backend can be swapped
//! later — e.g. to libSQL for cross-device sync — without touching callers. The
//! on-disk file format is SQLite-compatible, so that swap needs no data
//! migration.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::fuzzy;

/// Ranking class bases. A row's class (where the query matched) dominates its
/// score — the bases are spaced wider than the largest reachable detail +
/// frecency bonus — so a host-prefix match always outranks a mid-URL one, and
/// frecency only reorders rows *within* a class. See [`History::query`].
const CLASS_PREFIX: i32 = 4000;
const CLASS_HOST: i32 = 3000;
const CLASS_URL: i32 = 2000;
const CLASS_TITLE: i32 = 1000;

/// How many rows a query fuzzy-scores. We take the most-frecent rows — the ones
/// actually worth completing to — and score those in memory. A personal history
/// rarely holds more genuinely-relevant candidates than this, and the bound
/// keeps the per-keystroke cost flat as history grows without end.
const SCAN_LIMIT: u32 = 2000;

/// One ranked autocomplete row from history. Serialized to JSON across the FFI
/// boundary (see `browser_query_suggestions` in the ffi crate).
///
/// `display_url` is the pretty, matched-against form of `url` (scheme + `www.`
/// dropped); `title_matches` / `url_matches` are the character indices in
/// `title` and `display_url` the query fuzzy-matched, so the native layer can
/// highlight exactly those characters. `url` remains the real navigation target.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySuggestion {
    pub url: String,
    pub title: String,
    pub host: String,
    pub display_url: String,
    pub title_matches: Vec<u32>,
    pub url_matches: Vec<u32>,
}

/// A history row pulled for scoring (private to the ranking pass).
struct Candidate {
    url: String,
    title: String,
    host: String,
    visit_count: i64,
    last_visit: i64,
}

/// One history-page row: a visited page with its recency and frequency, for the
/// native History surface. Distinct from [`HistorySuggestion`] (the autocomplete
/// shape) because the page needs the visit timestamp for day-grouping. Serialized
/// to JSON across the FFI boundary (see `browser_history_list` in the ffi crate).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub host: String,
    /// Unix seconds of the most recent visit.
    pub last_visit: i64,
    pub visit_count: i64,
}

/// The store — one SQLite connection with the schema applied on open.
pub struct History {
    conn: Connection,
}

impl History {
    fn from_conn(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// Open (creating if needed) the store at `path`.
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        Self::from_conn(Connection::open(path)?)
    }

    /// Record a visit to `url`: insert it, or bump its visit count and recency.
    /// Non-http(s) URLs (`about:`, `data:`, `chrome:`, blank) never enter
    /// history because [`host_of`] rejects them.
    fn record_visit(&self, url: &str) -> rusqlite::Result<()> {
        let Some(host) = host_of(url) else {
            return Ok(());
        };
        self.conn.execute(
            "INSERT INTO history (url, host, visit_count, last_visit)
                 VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(url) DO UPDATE SET
                 visit_count = visit_count + 1,
                 last_visit  = ?3",
            params![url, host, now()],
        )?;
        Ok(())
    }

    /// Update the stored title for `url` (no-op if the row does not exist yet).
    fn update_title(&self, url: &str, title: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE history SET title = ?2 WHERE url = ?1",
            params![url, title],
        )?;
        Ok(())
    }

    /// Cache (or replace) the favicon PNG bytes for `host`.
    fn store_favicon(&self, host: &str, png: &[u8]) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO favicons (host, png, updated) VALUES (?1, ?2, ?3)
             ON CONFLICT(host) DO UPDATE SET png = ?2, updated = ?3",
            params![host, png, now()],
        )?;
        Ok(())
    }

    /// The cached favicon PNG bytes for `host`, if any.
    fn favicon(&self, host: &str) -> rusqlite::Result<Option<Vec<u8>>> {
        self.conn
            .query_row(
                "SELECT png FROM favicons WHERE host = ?1",
                params![host],
                |row| row.get(0),
            )
            .optional()
    }

    /// Up to `limit` fuzzy-ranked autocomplete rows for `query`.
    ///
    /// Ranking mirrors a modern browser's omnibox: the query is fuzzy-matched
    /// (subsequence, boundary- and run-aware — see [`crate::fuzzy`]) against each
    /// candidate's host, display URL, and title, and each row is bucketed into a
    /// *class* by where it matched — a host **prefix** (also what drives inline
    /// autocomplete) ranks above a host fuzzy-match, above a URL match, above a
    /// title-only match. The class dominates the score, so match quality beats
    /// frecency; within a class, the fuzzy detail and a log-scaled frecency
    /// bonus order the rows (a site you visit often floats up). An empty query
    /// returns the most-frecent pages.
    fn query(&self, query: &str, limit: u32) -> rusqlite::Result<Vec<HistorySuggestion>> {
        let query = query.trim();
        if query.is_empty() {
            return self.top_frecent(limit);
        }

        let now = now();
        let normalized = strip_scheme_www(&query.to_lowercase());
        let mut scored: Vec<(i32, i64, i64, HistorySuggestion)> = Vec::new();

        for c in self.candidates(SCAN_LIMIT)? {
            let display = display_url(&c.url);
            let host = strip_www(&c.host);
            let title_m = (!c.title.is_empty())
                .then(|| fuzzy::fuzzy_match(query, &c.title))
                .flatten();
            let url_m = fuzzy::fuzzy_match(query, &display);
            let host_m = fuzzy::fuzzy_match(query, host);
            let is_prefix =
                !normalized.is_empty() && display.to_lowercase().starts_with(&normalized);

            // Pick the class (and its in-class detail score) by match quality.
            let (base, detail) = if is_prefix {
                (CLASS_PREFIX, url_m.as_ref().map_or(0, |m| m.score))
            } else if let Some(m) = &host_m {
                (CLASS_HOST, m.score)
            } else if let Some(m) = &url_m {
                (CLASS_URL, m.score)
            } else if let Some(m) = &title_m {
                (CLASS_TITLE, m.score)
            } else {
                continue; // no field matched — drop it
            };

            let score = base + detail + frecency_bonus(c.visit_count, c.last_visit, now)
                - specificity_penalty(&display, c.visit_count);
            scored.push((
                score,
                c.visit_count,
                c.last_visit,
                HistorySuggestion {
                    url: c.url,
                    title: c.title,
                    host: c.host,
                    display_url: display,
                    title_matches: title_m.map(|m| m.indices).unwrap_or_default(),
                    url_matches: url_m.map(|m| m.indices).unwrap_or_default(),
                },
            ));
        }

        // Score desc, then frecency signals as a stable tie-break.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
        Ok(scored
            .into_iter()
            .take(limit as usize)
            .map(|(_, _, _, s)| s)
            .collect())
    }

    /// The `cap` most-frecent history rows, for fuzzy scoring or (empty-query)
    /// direct display.
    fn candidates(&self, cap: u32) -> rusqlite::Result<Vec<Candidate>> {
        let mut stmt = self.conn.prepare(
            "SELECT url, title, host, visit_count, last_visit FROM history
             ORDER BY visit_count DESC, last_visit DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![cap], |row| {
            Ok(Candidate {
                url: row.get(0)?,
                title: row.get(1)?,
                host: row.get(2)?,
                visit_count: row.get(3)?,
                last_visit: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// The `limit` most-frecent pages, as suggestions with no highlight ranges
    /// (there is no query to highlight). Sorted by frecency — visit frequency
    /// weighted by recency — so the zero-state omnibox surfaces pages the user
    /// actually returns to, not merely the highest raw visit count.
    fn top_frecent(&self, limit: u32) -> rusqlite::Result<Vec<HistorySuggestion>> {
        let now = now();
        let mut rows = self.candidates(SCAN_LIMIT)?;
        rows.sort_by(|a, b| {
            let fa = frecency_bonus(a.visit_count, a.last_visit, now);
            let fb = frecency_bonus(b.visit_count, b.last_visit, now);
            fb.cmp(&fa)
                .then(b.last_visit.cmp(&a.last_visit))
                .then(b.visit_count.cmp(&a.visit_count))
        });
        Ok(rows
            .into_iter()
            .take(limit as usize)
            .map(|c| HistorySuggestion {
                display_url: display_url(&c.url),
                url: c.url,
                title: c.title,
                host: c.host,
                title_matches: Vec::new(),
                url_matches: Vec::new(),
            })
            .collect())
    }

    /// Up to `limit` history rows (skipping the first `offset`) matching `query`
    /// — a case-insensitive substring of the URL or title — in reverse-
    /// chronological order (most recent visit first). An empty `query` lists all
    /// history. Unlike [`query`](Self::query), which ranks by frecency for
    /// autocomplete, this orders strictly by recency for the History page.
    fn list(&self, query: &str, limit: u32, offset: u32) -> rusqlite::Result<Vec<HistoryEntry>> {
        let like = format!("%{}%", escape_like(query));
        let mut stmt = self.conn.prepare(
            "SELECT url, title, host, last_visit, visit_count FROM history
             WHERE url LIKE ?1 ESCAPE '\\' OR title LIKE ?1 ESCAPE '\\'
             ORDER BY last_visit DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(params![like, limit, offset], |row| {
            Ok(HistoryEntry {
                url: row.get(0)?,
                title: row.get(1)?,
                host: row.get(2)?,
                last_visit: row.get(3)?,
                visit_count: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Delete the history row for `url`. Returns whether a row was removed.
    /// Leaves the host's cached favicon untouched (other URLs may still use it;
    /// orphans are harmless cache).
    fn delete(&self, url: &str) -> rusqlite::Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM history WHERE url = ?1", params![url])?;
        Ok(n > 0)
    }

    /// Delete history rows last visited at or after `since` (Unix seconds);
    /// `since == 0` clears all history. Returns the number of rows removed.
    fn clear_since(&self, since: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM history WHERE last_visit >= ?1", params![since])
    }
}

/// `PRAGMA journal_mode=WAL` rides in the batch so it applies on every open;
/// `execute_batch` discards the row it returns (and SQLite ignores WAL for the
/// in-memory databases used by the tests).
const SCHEMA: &str = "
    PRAGMA journal_mode=WAL;
    CREATE TABLE IF NOT EXISTS history (
        url         TEXT PRIMARY KEY,
        title       TEXT NOT NULL DEFAULT '',
        host        TEXT NOT NULL,
        visit_count INTEGER NOT NULL DEFAULT 0,
        last_visit  INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS favicons (
        host    TEXT PRIMARY KEY,
        png     BLOB NOT NULL,
        updated INTEGER NOT NULL
    );
";

/// Extract the lowercased host from an http(s) URL. Returns `None` for every
/// other scheme (`about:`, `data:`, `chrome:`, `file:`, blank) so they never
/// enter history and never get a favicon row.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    // Authority is everything up to the first '/', '?' or '#'.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    // Drop any `userinfo@` prefix, then any `:port` suffix.
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// Escape the SQL `LIKE` metacharacters so a typed `%` or `_` matches literally.
/// Pairs with `ESCAPE '\'` in the query.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// The pretty, matched-against form of a URL: scheme and a leading `www.`
/// dropped, and any single trailing slash removed
/// (`https://www.example.com/` → `example.com`). The suggestion shows this and
/// fuzzy-matching + inline autocomplete run against it, so highlight indices and
/// the completion text stay aligned with what the user sees.
pub fn display_url(url: &str) -> String {
    let mut s = url;
    for scheme in ["https://", "http://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            s = rest;
            break;
        }
    }
    s = s.strip_prefix("www.").unwrap_or(s);
    s.strip_suffix('/').unwrap_or(s).to_string()
}

/// A host with a leading `www.` removed (for host matching, which should treat
/// `www.example.com` and `example.com` alike).
fn strip_www(host: &str) -> &str {
    host.strip_prefix("www.").unwrap_or(host)
}

/// Strip a leading scheme and `www.` from an already-lowercased query, so typing
/// `https://git` or `www.git` still prefix-matches a stored `github.com`.
fn strip_scheme_www(query: &str) -> String {
    let mut s = query;
    for scheme in ["https://", "http://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            s = rest;
            break;
        }
    }
    strip_www(s).to_string()
}

/// Penalty for URLs deeper than the bare host — keeps `youtube.com` above
/// `youtube.com/watch?v=…` within the same match class. Waived once a deep URL
/// is visited often enough that it is clearly the user's habitual destination.
fn specificity_penalty(display: &str, visit_count: i64) -> i32 {
    const FREQUENT_THRESHOLD: i64 = 15;
    if visit_count >= FREQUENT_THRESHOLD {
        return 0;
    }
    let segments = display.split('/').count().saturating_sub(1);
    (segments as i32) * 250
}

/// A frecency bonus: visit frequency weighted by how recently the page was seen,
/// log-scaled so a heavily-visited site lifts above a rarely-visited one without
/// ever overpowering match quality (the class bases dominate). Recency buckets
/// mirror the coarse weighting browsers use.
fn frecency_bonus(visit_count: i64, last_visit: i64, now: i64) -> i32 {
    let age_days = (now - last_visit).max(0) / 86_400;
    let weight = match age_days {
        0..=3 => 100,
        4..=13 => 70,
        14..=30 => 50,
        31..=90 => 30,
        _ => 10,
    };
    let frecency = visit_count.max(0) as f64 * weight as f64;
    ((frecency + 1.0).ln() * 40.0) as i32
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ─── Process-wide store ──────────────────────────────────────────────────────

static STORE: OnceLock<Mutex<History>> = OnceLock::new();

/// Open the process-wide store at `path`. Call once during initialization.
///
/// A failure to open is logged and leaves the store absent — every operation
/// below then becomes a silent no-op, so the browser runs without autocomplete
/// rather than aborting.
pub fn init(path: &str) {
    match History::open(path) {
        Ok(history) => {
            if STORE.set(Mutex::new(history)).is_err() {
                eprintln!("[history] store already initialized; ignoring re-init");
            }
        }
        Err(err) => eprintln!("[history] failed to open {path}: {err}"),
    }
}

fn with_store<T>(f: impl FnOnce(&History) -> rusqlite::Result<T>) -> Option<T> {
    let store = STORE.get()?.lock().ok()?;
    match f(&store) {
        Ok(value) => Some(value),
        Err(err) => {
            eprintln!("[history] operation failed: {err}");
            None
        }
    }
}

/// Record a page visit. Off-scheme URLs are ignored.
pub fn record_visit(url: &str) {
    with_store(|h| h.record_visit(url));
}

/// Set the title for an already-recorded page.
pub fn update_title(url: &str, title: &str) {
    with_store(|h| h.update_title(url, title));
}

/// In-memory favicons for private windows and other sessions that must not
/// persist icon bytes to disk.
static SESSION_FAVICONS: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();

fn session_favicons() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    SESSION_FAVICONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Cache a favicon PNG for `host` without writing to the history database.
pub fn store_session_favicon(host: &str, png: &[u8]) {
    if host.is_empty() {
        return;
    }
    session_favicons()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(host.to_string(), png.to_vec());
}

/// Cache the favicon PNG for a host.
pub fn store_favicon(host: &str, png: &[u8]) {
    store_session_favicon(host, png);
    with_store(|h| h.store_favicon(host, png));
}

/// Frecency-ranked suggestions for a typed query (empty vec if the store is
/// absent or the query fails).
pub fn query(query: &str, limit: u32) -> Vec<HistorySuggestion> {
    with_store(|h| h.query(query, limit)).unwrap_or_default()
}

/// History-page rows for `query`, most-recent first (empty vec if the store is
/// absent). An empty `query` lists everything; `limit`/`offset` paginate.
pub fn list(query: &str, limit: u32, offset: u32) -> Vec<HistoryEntry> {
    with_store(|h| h.list(query, limit, offset)).unwrap_or_default()
}

/// Delete the history entry for `url`. Returns whether a row was removed.
pub fn delete(url: &str) -> bool {
    with_store(|h| h.delete(url)).unwrap_or(false)
}

/// Clear history entries last visited at or after `since` (Unix seconds); `0`
/// clears everything. Returns the number of rows removed.
pub fn clear_since(since: i64) -> usize {
    with_store(|h| h.clear_since(since)).unwrap_or(0)
}

/// Cached favicon PNG bytes for a host, if present.
pub fn favicon(host: &str) -> Option<Vec<u8>> {
    session_favicons()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(host)
        .cloned()
        .or_else(|| with_store(|h| h.favicon(host)).flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> History {
        History::from_conn(Connection::open_in_memory().unwrap()).unwrap()
    }

    #[test]
    fn host_of_parses_http_and_https() {
        assert_eq!(
            host_of("https://example.com/path"),
            Some("example.com".into())
        );
        assert_eq!(host_of("http://Example.COM"), Some("example.com".into()));
        assert_eq!(
            host_of("https://user:pass@host.io:8080/x"),
            Some("host.io".into())
        );
        assert_eq!(host_of("https://a.com?q=1"), Some("a.com".into()));
        assert_eq!(host_of("about:blank"), None);
        assert_eq!(host_of("data:text/html,hi"), None);
        assert_eq!(host_of("chrome://settings"), None);
        assert_eq!(host_of(""), None);
    }

    #[test]
    fn records_and_ranks_by_visit_count() {
        let h = store();
        h.record_visit("https://a.com/").unwrap();
        h.record_visit("https://b.com/").unwrap();
        h.record_visit("https://b.com/").unwrap(); // b visited twice
        let out = h.query("", 10).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].url, "https://b.com/"); // higher visit_count ranks first
    }

    #[test]
    fn query_matches_url_and_title_case_insensitively() {
        let h = store();
        let url = "https://github.com/rust-lang/rust";
        h.record_visit(url).unwrap();
        h.update_title(url, "The Rust Language").unwrap();
        assert_eq!(h.query("github", 10).unwrap().len(), 1); // url match
        assert_eq!(h.query("GITHUB", 10).unwrap().len(), 1); // case-insensitive
        assert_eq!(h.query("language", 10).unwrap().len(), 1); // title match
        assert!(h.query("python", 10).unwrap().is_empty());
    }

    #[test]
    fn ignores_non_http_urls() {
        let h = store();
        h.record_visit("about:blank").unwrap();
        h.record_visit("data:text/html,x").unwrap();
        assert!(h.query("", 10).unwrap().is_empty());
    }

    #[test]
    fn favicon_roundtrips_and_upserts() {
        let h = store();
        assert_eq!(h.favicon("example.com").unwrap(), None);
        h.store_favicon("example.com", &[1, 2, 3]).unwrap();
        assert_eq!(h.favicon("example.com").unwrap(), Some(vec![1, 2, 3]));
        h.store_favicon("example.com", &[4, 5]).unwrap();
        assert_eq!(h.favicon("example.com").unwrap(), Some(vec![4, 5]));
    }

    #[test]
    fn punctuation_query_does_not_match_everything() {
        let h = store();
        h.record_visit("https://ab.com/").unwrap();
        // A typed '%' is not a subsequence of the candidate, so it matches
        // nothing (the old LIKE matcher had to escape it to avoid wildcarding).
        assert!(h.query("%", 10).unwrap().is_empty());
    }

    #[test]
    fn fuzzy_matches_non_substring_subsequence() {
        let h = store();
        h.record_visit("https://github.com/").unwrap();
        // "gh" is a subsequence but not a substring of "github.com" — the old
        // LIKE matcher missed it; the fuzzy matcher finds it.
        let out = h.query("gh", 10).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].host, "github.com");
    }

    #[test]
    fn host_prefix_outranks_deeper_match() {
        let h = store();
        // A rarely-visited host-prefix match vs a frequently-visited page where
        // the query appears only deep in the path.
        h.record_visit("https://github.com/").unwrap();
        for _ in 0..5 {
            h.record_visit("https://stackoverflow.com/questions/git-rebase")
                .unwrap();
        }
        let out = h.query("git", 10).unwrap();
        assert_eq!(out[0].host, "github.com"); // match class beats frecency
    }

    #[test]
    fn origin_url_outranks_deep_path_within_a_class() {
        let h = store();
        for _ in 0..3 {
            h.record_visit("https://www.youtube.com/watch?v=abc123")
                .unwrap();
        }
        h.record_visit("https://www.youtube.com/").unwrap();
        let out = h.query("you", 10).unwrap();
        assert_eq!(out[0].display_url, "youtube.com");
    }

    #[test]
    fn heavily_visited_deep_url_can_beat_origin() {
        let h = store();
        h.record_visit("https://www.youtube.com/").unwrap();
        for _ in 0..30 {
            h.record_visit("https://www.youtube.com/watch?v=daily")
                .unwrap();
        }
        let out = h.query("you", 10).unwrap();
        assert_eq!(out[0].display_url, "youtube.com/watch?v=daily");
    }

    #[test]
    fn frecency_orders_within_a_class() {
        let h = store();
        h.record_visit("https://git-scm.com/").unwrap();
        for _ in 0..5 {
            h.record_visit("https://github.com/").unwrap();
        }
        // Both are host-prefix matches for "git"; the more-visited one wins.
        let out = h.query("git", 10).unwrap();
        assert_eq!(out[0].host, "github.com");
    }

    #[test]
    fn returns_display_url_and_match_indices() {
        let h = store();
        h.record_visit("https://github.com/").unwrap();
        let out = h.query("git", 10).unwrap();
        assert_eq!(out[0].display_url, "github.com"); // scheme + trailing slash gone
        assert_eq!(out[0].url_matches, vec![0, 1, 2]); // "git" highlighted
    }

    #[test]
    fn display_url_strips_scheme_www_and_trailing_slash() {
        assert_eq!(display_url("https://www.example.com/"), "example.com");
        assert_eq!(display_url("http://example.com/a/b"), "example.com/a/b");
        assert_eq!(display_url("https://example.com/?q=1"), "example.com/?q=1");
    }

    /// Record `url` then stamp its visit time explicitly — `record_visit` uses
    /// the wall clock, so tests that assert on ordering/cutoffs set it directly.
    fn visit_at(h: &History, url: &str, at: i64) {
        h.record_visit(url).unwrap();
        h.conn
            .execute(
                "UPDATE history SET last_visit = ?2 WHERE url = ?1",
                params![url, at],
            )
            .unwrap();
    }

    #[test]
    fn list_is_reverse_chronological_and_paginates() {
        let h = store();
        visit_at(&h, "https://a.com/", 100);
        visit_at(&h, "https://b.com/", 300);
        visit_at(&h, "https://c.com/", 200);
        let urls: Vec<_> = h
            .list("", 10, 0)
            .unwrap()
            .into_iter()
            .map(|e| e.url)
            .collect();
        assert_eq!(urls, ["https://b.com/", "https://c.com/", "https://a.com/"]);
        // offset skips the most-recent row.
        let page = h.list("", 1, 1).unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].url, "https://c.com/");
    }

    #[test]
    fn list_filters_by_query_and_carries_metadata() {
        let h = store();
        visit_at(&h, "https://github.com/x", 100);
        h.update_title("https://github.com/x", "Code").unwrap();
        visit_at(&h, "https://example.com/", 200);
        assert_eq!(h.list("github", 10, 0).unwrap().len(), 1);
        assert_eq!(h.list("code", 10, 0).unwrap().len(), 1); // title match
        let all = h.list("", 10, 0).unwrap();
        assert_eq!(all.len(), 2);
        let gh = all.iter().find(|e| e.host == "github.com").unwrap();
        assert_eq!(gh.title, "Code");
        assert_eq!(gh.last_visit, 100);
        assert_eq!(gh.visit_count, 1);
    }

    #[test]
    fn delete_removes_one_entry() {
        let h = store();
        h.record_visit("https://a.com/").unwrap();
        h.record_visit("https://b.com/").unwrap();
        assert!(h.delete("https://a.com/").unwrap());
        assert!(!h.delete("https://a.com/").unwrap()); // already gone
        let rest = h.list("", 10, 0).unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].url, "https://b.com/");
    }

    #[test]
    fn clear_since_respects_cutoff_and_clears_all() {
        let h = store();
        visit_at(&h, "https://old.com/", 100);
        visit_at(&h, "https://new.com/", 500);
        // Cutoff at 300 removes only the newer row.
        assert_eq!(h.clear_since(300).unwrap(), 1);
        let rest = h.list("", 10, 0).unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].url, "https://old.com/");
        // Zero clears everything remaining.
        assert_eq!(h.clear_since(0).unwrap(), 1);
        assert!(h.list("", 10, 0).unwrap().is_empty());
    }
}
