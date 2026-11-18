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
//! sees no cross-thread contention - the lock is defensive, not a hot path.
//!
//! The store sits behind this small surface (`record_visit`, `update_title`,
//! `store_favicon`, `query`, `list`, `delete`, `clear_since`, `favicon`)
//! precisely so the backend can be swapped
//! later - e.g. to libSQL for cross-device sync - without touching callers. The
//! on-disk file format is SQLite-compatible, so that swap needs no data
//! migration.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// How many rows a query fuzzy-scores. We take the most-frecent rows - the ones
/// actually worth completing to - and score those in memory. A personal history
/// rarely holds more genuinely-relevant candidates than this, and the bound
/// keeps the per-keystroke cost flat as history grows without end.
const SCAN_LIMIT: u32 = 2000;

/// A history page with the signals the omnibox ranks on (ADR-0071).
#[derive(Debug, Clone, PartialEq)]
pub struct OmniboxPage {
    pub url: String,
    pub title: String,
    pub host: String,
    pub visit_count: i64,
    pub last_visit: i64,
    pub typed_count: i64,
}

/// A learned pick: typing `input` (or a longer string starting with it) led
/// the user to `url`.
#[derive(Debug, Clone, PartialEq)]
pub struct Shortcut {
    pub input: String,
    pub url: String,
    pub use_count: f64,
    pub last_used: i64,
}

/// `count` decayed by 0.975 per day since `last` (Firefox adaptive history).
pub fn decay(count: f64, last: i64, now: i64) -> f64 {
    let days = ((now - last).max(0) / 86_400) as i32;
    count * 0.975f64.powi(days)
}

/// One history-page row: a visited page with its recency and frequency, for the
/// native History surface. Distinct from [`OmniboxPage`] (the omnibox's
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

/// The store - one SQLite connection with the schema applied on open.
pub struct History {
    conn: Connection,
}

impl History {
    fn from_conn(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(SCHEMA)?;
        // Added for the omnibox (ADR-0071): how often the user typed this URL
        // (or accepted its inline completion). Older stores lack the column.
        let has_typed: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('history') WHERE name = 'typed_count'")?
            .exists([])?;
        if !has_typed {
            conn.execute_batch(
                "ALTER TABLE history ADD COLUMN typed_count INTEGER NOT NULL DEFAULT 0",
            )?;
        }
        Ok(Self { conn })
    }

    /// The `cap` most-frecent pages with everything the omnibox ranks on.
    fn omnibox_candidates(&self, cap: u32) -> rusqlite::Result<Vec<OmniboxPage>> {
        let mut stmt = self.conn.prepare(
            "SELECT url, title, host, visit_count, last_visit, typed_count FROM history
             ORDER BY visit_count DESC, last_visit DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![cap], |row| {
            Ok(OmniboxPage {
                url: row.get(0)?,
                title: row.get(1)?,
                host: row.get(2)?,
                visit_count: row.get(3)?,
                last_visit: row.get(4)?,
                typed_count: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    /// Every learned (typed input -> chosen URL) pair, dropping the ones unused
    /// for about 90 days (Firefox adaptive history's prune point).
    fn shortcuts(&self) -> rusqlite::Result<Vec<Shortcut>> {
        let cutoff = now() - 90 * 86_400;
        self.conn.execute(
            "DELETE FROM input_history WHERE last_used < ?1",
            params![cutoff],
        )?;
        let mut stmt = self
            .conn
            .prepare("SELECT input, url, use_count, last_used FROM input_history")?;
        let rows = stmt.query_map([], |row| {
            Ok(Shortcut {
                input: row.get(0)?,
                url: row.get(1)?,
                use_count: row.get(2)?,
                last_used: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Learn from a pick: `input` led to `url`. `use_count` decays by 0.975 per
    /// idle day, then `x 0.9 + 1` (approaches 10), as Firefox adaptive history
    /// does. `typed` also counts the URL as typed, which gates inline
    /// completion.
    fn note_choice(&self, input: &str, url: &str, typed: bool) -> rusqlite::Result<()> {
        let input = input.trim().to_lowercase();
        let now = now();
        if !input.is_empty() {
            let prior: Option<(f64, i64)> = self
                .conn
                .query_row(
                    "SELECT use_count, last_used FROM input_history WHERE input = ?1 AND url = ?2",
                    params![input, url],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let decayed = prior.map_or(0.0, |(count, last)| decay(count, last, now));
            self.conn.execute(
                "INSERT INTO input_history (input, url, use_count, last_used) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(input, url) DO UPDATE SET use_count = ?3, last_used = ?4",
                params![input, url, decayed * 0.9 + 1.0, now],
            )?;
        }
        if typed {
            if let Some(host) = host_of(url) {
                self.conn.execute(
                    "INSERT INTO history (url, host, visit_count, last_visit, typed_count)
                         VALUES (?1, ?2, 0, ?3, 1)
                     ON CONFLICT(url) DO UPDATE SET typed_count = typed_count + 1",
                    params![url, host, now],
                )?;
            }
        }
        Ok(())
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

    /// Up to `limit` history rows (skipping the first `offset`) matching `query`
    /// (a case-insensitive substring of the URL or title) in reverse-
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
    CREATE TABLE IF NOT EXISTS input_history (
        input     TEXT NOT NULL,
        url       TEXT NOT NULL,
        use_count REAL NOT NULL,
        last_used INTEGER NOT NULL,
        PRIMARY KEY (input, url)
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
pub(crate) fn strip_scheme_www(query: &str) -> String {
    let mut s = query;
    for scheme in ["https://", "http://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            s = rest;
            break;
        }
    }
    strip_www(s).to_string()
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
/// A failure to open is logged and leaves the store absent - every operation
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

/// Pages and learned picks for the omnibox ranker (empty if the store is
/// unavailable).
pub fn omnibox_inputs() -> (Vec<OmniboxPage>, Vec<Shortcut>) {
    with_store(|h| Ok((h.omnibox_candidates(SCAN_LIMIT)?, h.shortcuts()?))).unwrap_or_default()
}

/// Learn that typing `input` led the user to `url`; `typed` also counts it as
/// a typed URL. See [`History::note_choice`].
pub fn note_choice(input: &str, url: &str, typed: bool) {
    with_store(|h| h.note_choice(input, url, typed));
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
    fn note_choice_learns_and_counts_typed_urls() {
        let h = store();
        h.record_visit("https://github.com/").unwrap();
        h.note_choice("gi", "https://github.com/", true).unwrap();
        h.note_choice("GI ", "https://github.com/", true).unwrap();
        let pages = h.omnibox_candidates(10).unwrap();
        assert_eq!(pages[0].typed_count, 2);
        let shortcuts = h.shortcuts().unwrap();
        assert_eq!(shortcuts.len(), 1);
        assert_eq!(shortcuts[0].input, "gi");
        assert!((shortcuts[0].use_count - 1.9).abs() < 1e-9);
        // A typed URL with no visit yet still gets a row to count on.
        h.note_choice("", "https://new.example/", true).unwrap();
        assert!(h
            .omnibox_candidates(10)
            .unwrap()
            .iter()
            .any(|p| p.url == "https://new.example/"));
    }

    #[test]
    fn decay_halves_in_about_four_weeks() {
        let d = decay(10.0, 0, 28 * 86_400);
        assert!((4.8..5.0).contains(&d), "{d}");
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
        let out = h.omnibox_candidates(10).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].url, "https://b.com/"); // higher visit_count ranks first
    }

    #[test]
    fn ignores_non_http_urls() {
        let h = store();
        h.record_visit("about:blank").unwrap();
        h.record_visit("data:text/html,x").unwrap();
        assert!(h.omnibox_candidates(10).unwrap().is_empty());
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
    fn display_url_strips_scheme_www_and_trailing_slash() {
        assert_eq!(display_url("https://www.example.com/"), "example.com");
        assert_eq!(display_url("http://example.com/a/b"), "example.com/a/b");
        assert_eq!(display_url("https://example.com/?q=1"), "example.com/?q=1");
    }

    /// Record `url` then stamp its visit time explicitly - `record_visit` uses
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
