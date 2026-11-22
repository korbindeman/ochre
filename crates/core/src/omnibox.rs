//! The omnibox ranker (ADR-0071): what the command bar and the address editor
//! suggest, which row Enter runs, and what the field completes inline.
//!
//! Modeled on Chromium's omnibox and Firefox's address bar:
//!
//! - One relevance scale. Search-what-you-typed is 1300, URL-what-you-typed
//!   1203, inline-completable history 1410+, other history at most 1399, remote
//!   search suggestions 800 and down.
//! - Only a row the input clearly leads to may be the default (Enter). Among
//!   those, the highest relevance wins; the default is shown first.
//! - The default is stable: when remote suggestions arrive, or the input is at
//!   least 4 characters long, the previous default stays if it still qualifies.
//! - Inline completion fills a site the user typed before (host: typed once;
//!   deeper URL: typed twice), only at the start of the host after the scheme
//!   or `www.`, never after a deletion or paste.
//! - It learns: picking a row for an input stores (input, URL); later inputs
//!   that start the same way rank that URL higher, and two picks make it
//!   completable inline.
//! - Rows that point at an open tab switch to it (Arc).
//!
//! [`rank`] is pure; [`query`] gathers history, learned picks and tabs.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::fuzzy;
use crate::history::{self, OmniboxPage, Shortcut};
use crate::tabs::TabId;

/// Relevance bands (Chromium's scale).
const INLINE_BASE: i32 = 1410;
const SEARCH_VERBATIM: i32 = 1300;
const URL_VERBATIM: i32 = 1203;
const HISTORY_MAX: i32 = 1399;
const SEARCH_FOR_URL_INPUT: i32 = 850;
const REMOTE_BASE: i32 = 800;
/// Rows shown, and per-group caps (default row excluded).
const MAX_ROWS: usize = 8;
const MAX_NAV: usize = 5;
const MAX_SEARCH: usize = 3;
/// A learned pick this strong (about two picks) may complete inline even if
/// the URL was never typed.
const SHORTCUT_INLINE: f64 = 1.85;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Request {
    pub text: String,
    /// The user just deleted, pasted, or edited mid-field: no inline fill.
    #[serde(default)]
    pub prevent_inline: bool,
    /// Remote search suggestions for this text, in the engine's order.
    #[serde(default)]
    pub remote: Vec<String>,
    /// The key of the default row the host is showing now.
    #[serde(default)]
    pub previous_default: Option<String>,
    /// True when only `remote` changed (no keystroke).
    #[serde(default)]
    pub async_pass: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Go to a page (or switch to its tab when `tab` is set).
    Url,
    /// Search the default engine for `query`.
    Search,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    /// Dedup and stability key: a normalized URL or `search:<query>`.
    pub key: String,
    pub kind: Kind,
    pub title: String,
    pub subtitle: Option<String>,
    /// The page to open (`Url`), empty for `Search`.
    pub url: String,
    /// The query to search (`Search`), empty for `Url`.
    pub query: String,
    pub host: Option<String>,
    /// An open tab showing this page: running the row switches to it.
    pub tab: Option<TabId>,
    pub title_matches: Vec<u32>,
    pub subtitle_matches: Vec<u32>,
    pub relevance: i32,
    #[serde(skip)]
    default_ok: bool,
    /// The host or URL prefix this row completes the field to, if any.
    #[serde(skip)]
    fill: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Response {
    pub rows: Vec<Row>,
    /// Whether `rows[0]` is the default (what Enter runs). False only for the
    /// empty-input list, where Enter does nothing.
    pub has_default: bool,
    /// The full field text with the inline completion, if any.
    pub inline: Option<String>,
}

/// What the ranker reads besides the request.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    pub pages: Vec<OmniboxPage>,
    pub shortcuts: Vec<Shortcut>,
    pub tabs: Vec<(TabId, String, String)>,
    pub now: i64,
}

/// Rank against the live history, learned picks and tabs.
pub fn query(request: &Request) -> Response {
    let (pages, shortcuts) = history::omnibox_inputs();
    let sources = Sources {
        pages,
        shortcuts,
        tabs: crate::tabs::omnibox_tabs(),
        now: now(),
    };
    rank(request, &sources)
}

/// Learn from a pick. `typed` marks a URL the user typed or accepted inline.
pub fn note_choice(input: &str, url: &str, typed: bool) {
    history::note_choice(input, url, typed);
}

/// One ranking pass. Pure.
pub fn rank(request: &Request, sources: &Sources) -> Response {
    let text = request.text.trim();
    if text.is_empty() {
        return zero_state(sources);
    }
    let q = text.to_lowercase();
    let single_token = !text.contains(char::is_whitespace);
    let typed_prefix = history::strip_scheme_www(&q);
    let inline_ok = single_token && !request.prevent_inline && !typed_prefix.is_empty();

    let mut rows: HashMap<String, Row> = HashMap::new();

    // What you typed.
    if let Some(url) = as_url(text) {
        add(
            &mut rows,
            Row {
                key: page_key(&url),
                kind: Kind::Url,
                title: history::display_url(&url),
                subtitle: None,
                host: history::host_of(&url),
                url,
                query: String::new(),
                tab: None,
                title_matches: Vec::new(),
                subtitle_matches: Vec::new(),
                relevance: URL_VERBATIM,
                default_ok: true,
                fill: None,
            },
        );
        add(&mut rows, search_row(text, SEARCH_FOR_URL_INPUT, false));
    } else {
        add(&mut rows, search_row(text, SEARCH_VERBATIM, true));
    }

    // Learned picks: the strongest stored input that starts with this text.
    let mut learned: HashMap<&str, f64> = HashMap::new();
    for s in &sources.shortcuts {
        if s.input.starts_with(&q) {
            let mut rank = history::decay(s.use_count, s.last_used, sources.now);
            if s.input == q {
                rank *= 2.0;
            }
            let best = learned.entry(s.url.as_str()).or_insert(0.0);
            *best = best.max(rank);
        }
    }

    // History pages. Every page that could complete inline is a candidate;
    // only the strongest one completes (below).
    let mut candidates: Vec<(String, &OmniboxPage, f64)> = Vec::new();
    for page in &sources.pages {
        let Some(m) = page_match(text, &q, &page.url, &page.title) else {
            if let Some(&strength) = learned.get(page.url.as_str()) {
                add(&mut rows, learned_only_row(page, strength));
            }
            continue;
        };
        let learned_strength = learned.get(page.url.as_str()).copied().unwrap_or(0.0);
        if inline_ok {
            if let Some(fill) = inline_fill(&typed_prefix, page, learned_strength) {
                candidates.push((fill, page, learned_strength));
            }
        }
        add(&mut rows, page_row(page, &m, sources.now, learned_strength));
    }
    if let Some((fill, page)) = inline_winner(&candidates, &sources.pages, sources.now) {
        add(&mut rows, completion_row(page, fill));
    }

    // Open tabs: switch instead of opening again.
    for (id, url, title) in &sources.tabs {
        let key = page_key(url);
        if let Some(row) = rows.get_mut(&key) {
            row.tab.get_or_insert(*id);
            continue;
        }
        if let Some(m) = page_match(text, &q, url, title) {
            add(&mut rows, tab_row(*id, url, title, &m));
        }
    }

    // Remote search suggestions.
    for (i, s) in request.remote.iter().enumerate() {
        let s = s.trim();
        if s.is_empty() || s.eq_ignore_ascii_case(text) {
            continue;
        }
        add(&mut rows, search_row(s, REMOTE_BASE - i as i32 * 10, false));
    }

    let mut rows: Vec<Row> = rows.into_values().collect();
    rows.sort_by(|a, b| b.relevance.cmp(&a.relevance).then(a.key.cmp(&b.key)));

    // The default: keep the previous one when allowed (stability), else the
    // highest-relevance row that may be default.
    let keep_previous = request.async_pass || text.chars().count() >= 4;
    let previous = request
        .previous_default
        .as_ref()
        .filter(|_| keep_previous)
        .and_then(|key| rows.iter().position(|r| &r.key == key && r.default_ok));
    let default = previous.or_else(|| rows.iter().position(|r| r.default_ok));
    let Some(default) = default else {
        return Response {
            rows: rows.into_iter().take(MAX_ROWS).collect(),
            has_default: false,
            inline: None,
        };
    };
    let first = rows.remove(default);
    let inline = first
        .fill
        .as_ref()
        .filter(|_| inline_ok)
        .map(|fill| format!("{text}{}", &fill[typed_prefix.len()..]));

    // Navigations above searches (the Arc feel); spare room goes to the other.
    let (nav, search): (Vec<Row>, Vec<Row>) = rows.into_iter().partition(|r| r.kind == Kind::Url);
    let room = MAX_ROWS - 1;
    let nav_take = nav
        .len()
        .min(MAX_NAV.max(room.saturating_sub(search.len().min(MAX_SEARCH))));
    let search_take = search.len().min(room - nav_take);
    let mut out = vec![first];
    out.extend(nav.into_iter().take(nav_take));
    out.extend(search.into_iter().take(search_take));
    Response {
        rows: out,
        has_default: true,
        inline,
    }
}

/// The empty field: the most frecent pages, open ones as tab switches. Enter
/// does nothing.
fn zero_state(sources: &Sources) -> Response {
    let tabs: HashMap<String, TabId> = sources
        .tabs
        .iter()
        .map(|(id, url, _)| (page_key(url), *id))
        .collect();
    let mut pages: Vec<&OmniboxPage> = sources.pages.iter().filter(|p| p.visit_count > 0).collect();
    pages.sort_by_key(|p| std::cmp::Reverse(frecency(p, sources.now)));
    let rows = pages
        .into_iter()
        .take(MAX_ROWS)
        .map(|p| {
            let mut row = page_row(p, &PageMatch::default(), sources.now, 0.0);
            row.tab = tabs.get(&row.key).copied();
            row
        })
        .collect();
    Response {
        rows,
        has_default: false,
        inline: None,
    }
}

/// Where the text matched a page, as a class and fuzzy detail.
#[derive(Default)]
struct PageMatch {
    /// 3 display prefix, 2 host, 1 URL, 0 title.
    class: i32,
    detail: i32,
    title: Vec<u32>,
    url: Vec<u32>,
}

fn page_match(text: &str, q: &str, url: &str, title: &str) -> Option<PageMatch> {
    let display = history::display_url(url);
    let host = history::host_of(url).unwrap_or_default();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let prefix = history::strip_scheme_www(q);
    let title_m = (!title.is_empty())
        .then(|| fuzzy::fuzzy_match(text, title))
        .flatten();
    let url_m = fuzzy::fuzzy_match(text, &display);
    let host_m = fuzzy::fuzzy_match(text, &host);
    let (class, detail) = if !prefix.is_empty() && display.to_lowercase().starts_with(&prefix) {
        (3, url_m.as_ref().map_or(0, |m| m.score))
    } else if let Some(m) = &host_m {
        (2, m.score)
    } else if let Some(m) = &url_m {
        (1, m.score)
    } else if let Some(m) = &title_m {
        (0, m.score)
    } else {
        return None;
    };
    Some(PageMatch {
        class,
        detail,
        title: title_m.map(|m| m.indices).unwrap_or_default(),
        url: url_m.map(|m| m.indices).unwrap_or_default(),
    })
}

/// Non-default history relevance, capped below inline completions: the match
/// class sets a band, frecency and learned picks move a row within it.
fn page_relevance(m: &PageMatch, frecency: i32, learned: f64, display: &str) -> i32 {
    let band = [900, 1000, 1100, 1300][m.class as usize];
    let depth = display.matches('/').count() as i32;
    let learned_bonus = (learned.min(10.0) * 8.0) as i32;
    (band + (m.detail / 4).min(20) + (frecency / 8).min(60) + learned_bonus - depth * 3)
        .min(HISTORY_MAX)
}

fn page_row(page: &OmniboxPage, m: &PageMatch, now: i64, learned: f64) -> Row {
    let display = history::display_url(&page.url);
    let has_title = !page.title.is_empty();
    Row {
        key: page_key(&page.url),
        kind: Kind::Url,
        title: if has_title {
            page.title.clone()
        } else {
            display.clone()
        },
        subtitle: has_title.then(|| display.clone()),
        url: page.url.clone(),
        query: String::new(),
        host: Some(page.host.clone()),
        tab: None,
        title_matches: if has_title {
            m.title.clone()
        } else {
            m.url.clone()
        },
        subtitle_matches: if has_title { m.url.clone() } else { Vec::new() },
        relevance: page_relevance(m, frecency(page, now), learned, &display),
        default_ok: false,
        fill: None,
    }
}

/// A page that matches only through a learned pick (typed "email", picked
/// gmail): ranked by how strong the pick is, never default.
fn learned_only_row(page: &OmniboxPage, strength: f64) -> Row {
    let mut row = page_row(page, &PageMatch::default(), 0, 0.0);
    row.relevance = (1000.0 + strength.min(10.0) * 30.0) as i32;
    row.relevance = row.relevance.min(HISTORY_MAX);
    row
}

fn tab_row(id: TabId, url: &str, title: &str, m: &PageMatch) -> Row {
    let display = history::display_url(url);
    let has_title = !title.is_empty();
    Row {
        key: page_key(url),
        kind: Kind::Url,
        title: if has_title {
            title.to_string()
        } else {
            display.clone()
        },
        subtitle: has_title.then(|| display.clone()),
        url: url.to_string(),
        query: String::new(),
        host: history::host_of(url),
        tab: Some(id),
        title_matches: if has_title {
            m.title.clone()
        } else {
            m.url.clone()
        },
        subtitle_matches: if has_title { m.url.clone() } else { Vec::new() },
        // Open and saved tabs are what an Arc user reaches for: a small lift.
        relevance: (page_relevance(m, 60, 0.0, &display) + 30).min(HISTORY_MAX),
        default_ok: false,
        fill: None,
    }
}

/// The host (or URL up to the next slash) this page completes `typed` to, if
/// the user typed it before often enough or picked it twice for this input.
fn inline_fill(typed: &str, page: &OmniboxPage, learned: f64) -> Option<String> {
    let display = history::display_url(&page.url).to_lowercase();
    if !display.starts_with(typed) || display.len() <= typed.len() {
        return None;
    }
    let host_len = display.find('/').unwrap_or(display.len());
    let trusted = |needed: i64| page.typed_count >= needed || learned >= SHORTCUT_INLINE;
    if typed.len() < host_len {
        // Inside the host: complete the host (origin first).
        trusted(1).then(|| display[..host_len].to_string())
    } else {
        // Past the host: complete to the next slash.
        let rest = &display[typed.len()..];
        let end = rest
            .char_indices()
            .skip(1)
            .find(|(_, c)| *c == '/')
            .map_or(display.len(), |(i, _)| typed.len() + i);
        trusted(2).then(|| display[..end].to_string())
    }
}

/// The one completion the field shows when several sites start with the input
/// ("you": youtube.com and yoursmileis.art). Like Firefox's origin autofill,
/// it is the completed site with the most use: visits and typings of every
/// page under that host or path, weighted by recency and never capped, so 960
/// visits beat 9. Learned picks for this input add to it. Ties go to the more
/// typed site, then the shorter fill. Returns the fill and the page to open.
fn inline_winner<'a>(
    candidates: &[(String, &'a OmniboxPage, f64)],
    pages: &[OmniboxPage],
    now: i64,
) -> Option<(String, &'a OmniboxPage)> {
    let mut fills: HashMap<&str, (f64, &'a OmniboxPage)> = HashMap::new();
    for (fill, page, learned) in candidates {
        let entry = fills.entry(fill.as_str()).or_insert((0.0, page));
        entry.0 = entry.0.max(*learned);
        // The page to open: the most typed one, then the most visited.
        if (page.typed_count, page.visit_count) > (entry.1.typed_count, entry.1.visit_count) {
            entry.1 = page;
        }
    }
    let mut best: Option<(f64, i64, &str, &'a OmniboxPage)> = None;
    for (fill, (learned, page)) in fills {
        let (mut use_score, mut typed) = (0.0, 0);
        for p in pages {
            if under(&history::display_url(&p.url).to_lowercase(), fill) {
                let weight = recency_weight(p.last_visit, now);
                use_score += (p.visit_count as f64 + 2.0 * p.typed_count as f64) * weight;
                typed += p.typed_count;
            }
        }
        let score = (use_score + 1.0).ln() + learned;
        let better = best.as_ref().is_none_or(|(s, t, f, _)| {
            (score, typed, std::cmp::Reverse(fill.len()))
                .partial_cmp(&(*s, *t, std::cmp::Reverse(f.len())))
                .is_some_and(|o| o.is_gt())
        });
        if better {
            best = Some((score, typed, fill, page));
        }
    }
    best.map(|(_, _, fill, page)| (fill.to_string(), page))
}

/// Whether `display` is `fill` itself or a page inside it.
fn under(display: &str, fill: &str) -> bool {
    display
        .strip_prefix(fill)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
}

/// The row an inline completion stands for: the completed host or path.
fn completion_row(page: &OmniboxPage, fill: String) -> Row {
    let display = history::display_url(&page.url);
    let scheme = if page.url.starts_with("http://") {
        "http://"
    } else {
        "https://"
    };
    let www = page.host.starts_with("www.") && !fill.starts_with("www.");
    let url = if fill.eq_ignore_ascii_case(&display) {
        page.url.clone()
    } else {
        format!("{scheme}{}{fill}", if www { "www." } else { "" })
    };
    let target = if url.ends_with('/') || fill.contains('/') {
        url
    } else {
        format!("{url}/")
    };
    let exact = fill.eq_ignore_ascii_case(&display);
    Row {
        key: page_key(&target),
        kind: Kind::Url,
        title: if exact && !page.title.is_empty() {
            page.title.clone()
        } else {
            fill.clone()
        },
        subtitle: (exact && !page.title.is_empty()).then(|| display.clone()),
        url: target,
        query: String::new(),
        host: Some(page.host.clone()),
        tab: None,
        title_matches: Vec::new(),
        subtitle_matches: Vec::new(),
        relevance: INLINE_BASE,
        default_ok: true,
        fill: Some(fill),
    }
}

fn search_row(query: &str, relevance: i32, default_ok: bool) -> Row {
    Row {
        key: format!("search:{}", query.to_lowercase()),
        kind: Kind::Search,
        title: query.to_string(),
        subtitle: None,
        url: String::new(),
        query: query.to_string(),
        host: None,
        tab: None,
        title_matches: Vec::new(),
        subtitle_matches: Vec::new(),
        relevance,
        default_ok,
        fill: None,
    }
}

fn add(rows: &mut HashMap<String, Row>, row: Row) {
    rows.entry(row.key.clone())
        .and_modify(|r| merge(r, &row))
        .or_insert(row);
}

/// Fold a duplicate into the kept row: best relevance and titles win, a tab
/// switch and default eligibility carry over.
fn merge(kept: &mut Row, other: &Row) {
    if other.relevance > kept.relevance {
        let tab = kept.tab.or(other.tab);
        let default_ok = kept.default_ok || other.default_ok;
        let fill = other.fill.clone().or_else(|| kept.fill.clone());
        *kept = other.clone();
        kept.tab = tab;
        kept.default_ok = default_ok;
        kept.fill = fill;
    } else {
        kept.tab = kept.tab.or(other.tab);
        kept.default_ok |= other.default_ok;
        if kept.fill.is_none() {
            kept.fill = other.fill.clone();
        }
    }
}

/// Dedup key: no scheme, no `www.`, no fragment, no trailing `/` or empty `?`.
fn page_key(url: &str) -> String {
    let url = url.split('#').next().unwrap_or(url);
    let mut key = history::strip_scheme_www(&url.to_lowercase());
    while key.ends_with('/') || key.ends_with('?') {
        key.pop();
    }
    key
}

/// The URL `text` names, if it reads as one: a scheme, `localhost`, an IP, or
/// a dotted host with no spaces.
fn as_url(text: &str) -> Option<String> {
    if text.contains(char::is_whitespace) {
        return None;
    }
    let lower = text.to_lowercase();
    if ["http://", "https://", "about:", "file://", "chrome://"]
        .iter()
        .any(|s| lower.starts_with(s))
    {
        return Some(text.to_string());
    }
    let host = lower.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    let dotted = host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host
            .rsplit('.')
            .next()
            .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphanumeric()));
    (dotted || host == "localhost").then(|| {
        let scheme = if host == "localhost" {
            "http://"
        } else {
            "https://"
        };
        format!("{scheme}{text}")
    })
}

/// Recency buckets (Chromium HQP's): how much a page's last visit still counts.
fn recency_weight(last_visit: i64, now: i64) -> f64 {
    match (now - last_visit).max(0) / 86_400 {
        0..=4 => 1.0,
        5..=14 => 0.7,
        15..=31 => 0.5,
        32..=90 => 0.3,
        _ => 0.1,
    }
}

/// Visit frequency weighted by recency, on a log scale. Not capped: a page
/// visited 960 times must outrank one visited 9 times.
fn frecency(page: &OmniboxPage, now: i64) -> i32 {
    let visits = page.visit_count as f64 + page.typed_count as f64 * 0.5;
    ((visits * 100.0 * recency_weight(page.last_visit, now) + 1.0).ln() * 40.0) as i32
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn page(url: &str, title: &str, visits: i64, typed: i64) -> OmniboxPage {
        OmniboxPage {
            url: url.into(),
            title: title.into(),
            host: history::host_of(url).unwrap(),
            visit_count: visits,
            last_visit: NOW - 3600,
            typed_count: typed,
        }
    }

    fn sources(pages: Vec<OmniboxPage>) -> Sources {
        Sources {
            pages,
            now: NOW,
            ..Sources::default()
        }
    }

    fn req(text: &str) -> Request {
        Request {
            text: text.into(),
            ..Request::default()
        }
    }

    #[test]
    fn plain_words_default_to_a_search() {
        let r = rank(&req("rust borrow checker"), &sources(vec![]));
        assert!(r.has_default);
        assert_eq!(r.rows[0].kind, Kind::Search);
        assert_eq!(r.rows[0].query, "rust borrow checker");
        assert_eq!(r.inline, None);
    }

    #[test]
    fn a_url_defaults_to_going_there() {
        let r = rank(&req("example.com/docs"), &sources(vec![]));
        assert_eq!(r.rows[0].kind, Kind::Url);
        assert_eq!(r.rows[0].url, "https://example.com/docs");
        assert!(r.rows.iter().any(|row| row.kind == Kind::Search));
        assert_eq!(
            rank(&req("localhost:3000"), &sources(vec![])).rows[0].url,
            "http://localhost:3000"
        );
    }

    #[test]
    fn a_typed_site_completes_inline_and_is_the_default() {
        let s = sources(vec![
            page("https://github.com/korbin/ochre", "ochre", 30, 0),
            page("https://github.com/", "GitHub", 5, 1),
        ]);
        let r = rank(&req("git"), &s);
        assert_eq!(r.inline.as_deref(), Some("github.com"));
        assert_eq!(r.rows[0].url, "https://github.com/");
    }

    #[test]
    fn the_most_used_site_wins_inline_when_several_share_the_prefix() {
        // Real history (Korbin's, 2026-11-22): "you" completed to a saved
        // store page's root because every typed site scored the same capped
        // frecency and the tie broke alphabetically. Accepting it then left a
        // learned pick for the wrong site, which must not outweigh 960 visits.
        let mut youtube = page("https://www.youtube.com/", "YouTube", 960, 358);
        youtube.last_visit = NOW - 600;
        let mut store = page("https://yoursmileis.art/b/bubblegum", "Bubblegum", 9, 7);
        store.last_visit = NOW - 18 * 86_400;
        let mut s = sources(vec![
            store,
            page("https://yoursmileis.art/", "", 3, 2),
            youtube,
            page("https://www.youtube.com/feed/history", "History", 19, 0),
        ]);
        let r = rank(&req("you"), &s);
        assert_eq!(r.inline.as_deref(), Some("youtube.com"));
        assert_eq!(r.rows[0].url, "https://www.youtube.com/");
        // Only one row completes; the other site is an ordinary suggestion.
        assert_eq!(
            r.rows
                .iter()
                .filter(|row| row.relevance >= INLINE_BASE)
                .count(),
            1
        );
        assert!(r
            .rows
            .iter()
            .any(|row| row.key.starts_with("yoursmileis.art")));

        s.shortcuts = vec![
            Shortcut {
                input: "you".into(),
                url: "https://yoursmileis.art/".into(),
                use_count: 1.9,
                last_used: NOW,
            },
            Shortcut {
                input: "you".into(),
                url: "https://www.youtube.com/".into(),
                use_count: 1.0,
                last_used: NOW,
            },
        ];
        assert_eq!(rank(&req("you"), &s).inline.as_deref(), Some("youtube.com"));
        // Typing past the shared prefix still reaches the other site.
        assert_eq!(
            rank(&req("yours"), &s).inline.as_deref(),
            Some("yoursmileis.art")
        );
    }

    #[test]
    fn an_untyped_site_does_not_complete_or_take_enter() {
        // Visited a lot but never typed: suggested, but Enter still searches.
        let s = sources(vec![page("https://github.com/", "GitHub", 50, 0)]);
        let r = rank(&req("git"), &s);
        assert_eq!(r.inline, None);
        assert_eq!(r.rows[0].kind, Kind::Search);
        assert_eq!(r.rows[1].url, "https://github.com/");
    }

    #[test]
    fn no_inline_completion_after_a_deletion_or_mid_word() {
        let s = sources(vec![page("https://github.com/", "GitHub", 5, 3)]);
        let mut request = req("git");
        request.prevent_inline = true;
        assert_eq!(rank(&request, &s).inline, None);
        // "hub" is inside the host, not its start.
        assert_eq!(rank(&req("hub"), &s).inline, None);
        // Two words never complete.
        assert_eq!(rank(&req("git hub"), &s).inline, None);
    }

    #[test]
    fn deeper_urls_complete_to_the_next_slash_after_two_typings() {
        let once = sources(vec![page(
            "https://news.ycombinator.com/item?id=1",
            "",
            3,
            1,
        )]);
        // Host completion needs one typing; path completion needs two.
        assert_eq!(
            rank(&req("news"), &once).inline.as_deref(),
            Some("news.ycombinator.com")
        );
        let docs = sources(vec![page("https://docs.rs/serde/latest", "serde", 4, 2)]);
        assert_eq!(
            rank(&req("docs.rs/s"), &docs).inline.as_deref(),
            Some("docs.rs/serde")
        );
        let docs_once = sources(vec![page("https://docs.rs/serde/latest", "serde", 4, 1)]);
        assert_eq!(rank(&req("docs.rs/s"), &docs_once).inline, None);
    }

    #[test]
    fn picking_a_page_twice_makes_it_complete_and_rank_first() {
        let mut s = sources(vec![page(
            "https://mail.google.com/mail/u/0/",
            "Inbox",
            20,
            0,
        )]);
        assert_eq!(rank(&req("mai"), &s).inline, None);
        s.shortcuts = vec![Shortcut {
            input: "mai".into(),
            url: "https://mail.google.com/mail/u/0/".into(),
            use_count: 1.9,
            last_used: NOW,
        }];
        let r = rank(&req("mai"), &s);
        assert_eq!(r.inline.as_deref(), Some("mail.google.com"));
        assert!(r.rows[0].url.starts_with("https://mail.google.com/"));
    }

    #[test]
    fn a_learned_pick_that_does_not_spell_the_url_ranks_high_without_taking_enter() {
        let mut s = sources(vec![page(
            "https://mail.google.com/mail/u/0/",
            "Inbox",
            20,
            0,
        )]);
        s.shortcuts = vec![Shortcut {
            input: "email".into(),
            url: "https://mail.google.com/mail/u/0/".into(),
            use_count: 5.0,
            last_used: NOW,
        }];
        let r = rank(&req("ema"), &s);
        assert_eq!(r.rows[0].kind, Kind::Search);
        assert_eq!(r.rows[1].url, "https://mail.google.com/mail/u/0/");
    }

    #[test]
    fn open_tabs_switch_instead_of_opening_again() {
        let mut s = sources(vec![page("https://github.com/", "GitHub", 5, 1)]);
        s.tabs = vec![
            (7, "https://github.com/".into(), "GitHub".into()),
            (8, "https://linear.app/team".into(), "Linear".into()),
        ];
        let r = rank(&req("git"), &s);
        assert_eq!(r.rows[0].tab, Some(7));
        let r = rank(&req("linear"), &s);
        assert!(r.rows.iter().any(|row| row.tab == Some(8)));
    }

    #[test]
    fn remote_suggestions_never_take_the_default_and_keep_it_stable() {
        let s = sources(vec![page("https://github.com/", "GitHub", 5, 1)]);
        let first = rank(&req("git"), &s);
        let mut later = req("git");
        later.remote = vec!["git".into(), "github copilot".into(), "gitlab".into()];
        later.async_pass = true;
        later.previous_default = Some(first.rows[0].key.clone());
        let r = rank(&later, &s);
        assert_eq!(r.rows[0].key, first.rows[0].key);
        assert!(r.rows.iter().any(|row| row.query == "github copilot"));
        assert!(!r
            .rows
            .iter()
            .any(|row| row.query == "git" && row.relevance == REMOTE_BASE));
    }

    #[test]
    fn short_input_does_not_stick_to_an_old_default() {
        let s = sources(vec![page("https://github.com/", "GitHub", 5, 1)]);
        let mut request = req("gi");
        request.previous_default = Some("search:g".into());
        assert_eq!(rank(&request, &s).rows[0].url, "https://github.com/");
    }

    #[test]
    fn duplicates_collapse_and_the_list_is_capped() {
        let mut pages = vec![
            page("https://example.com/", "Example", 5, 0),
            page("http://www.example.com/#top", "Example", 1, 0),
        ];
        for i in 0..20 {
            pages.push(page(
                &format!("https://example.com/p{i}"),
                "Example page",
                2,
                0,
            ));
        }
        let mut request = req("example");
        request.remote = (0..10).map(|i| format!("example {i}")).collect();
        let r = rank(&request, &sources(pages));
        assert!(r.rows.len() <= MAX_ROWS);
        let roots = r.rows.iter().filter(|row| row.key == "example.com").count();
        assert_eq!(roots, 1);
        // Navigations come before searches after the default row.
        let kinds: Vec<Kind> = r.rows[1..].iter().map(|row| row.kind).collect();
        let first_search = kinds
            .iter()
            .position(|k| *k == Kind::Search)
            .unwrap_or(kinds.len());
        assert!(kinds[first_search..].iter().all(|k| *k == Kind::Search));
    }

    #[test]
    fn non_ascii_urls_never_panic() {
        let s = sources(vec![page("https://example.com/ünï/çødé", "x", 3, 5)]);
        for text in ["example.com/", "example.com/ü", "exämple", "ü"] {
            let _ = rank(&req(text), &s);
        }
    }

    #[test]
    fn the_empty_field_lists_frecent_pages_and_enter_does_nothing() {
        let mut s = sources(vec![
            page("https://a.com/", "A", 9, 0),
            page("https://b.com/", "B", 2, 0),
        ]);
        s.tabs = vec![(3, "https://b.com/".into(), "B".into())];
        let r = rank(&req("  "), &s);
        assert!(!r.has_default);
        assert_eq!(r.rows[0].url, "https://a.com/");
        assert_eq!(r.rows[1].tab, Some(3));
    }

    #[test]
    fn the_default_is_always_one_the_input_leads_to() {
        // Property over a grid of inputs: a URL default is either the typed
        // URL or a completion of the typed prefix.
        let mut s = sources(vec![
            page("https://github.com/", "GitHub", 5, 2),
            page("https://gitlab.com/", "GitLab", 2, 1),
            page("https://news.ycombinator.com/", "HN", 9, 0),
        ]);
        s.shortcuts = vec![Shortcut {
            input: "hn".into(),
            url: "https://news.ycombinator.com/".into(),
            use_count: 4.0,
            last_used: NOW,
        }];
        for text in [
            "g",
            "gi",
            "git",
            "gitl",
            "hn",
            "news",
            "n",
            "x y",
            "example.org",
            "ycomb",
        ] {
            let r = rank(&req(text), &s);
            let first = &r.rows[0];
            match first.kind {
                Kind::Search => assert_eq!(first.query, text),
                Kind::Url => {
                    let typed = history::strip_scheme_www(&text.to_lowercase());
                    let display = history::display_url(&first.url).to_lowercase();
                    assert!(display.starts_with(&typed), "{text} -> {}", first.url);
                }
            }
        }
    }
}
