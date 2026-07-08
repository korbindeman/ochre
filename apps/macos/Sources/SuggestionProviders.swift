import Foundation

/// The concrete suggestion sources plugged into the command bar's autocomplete.
/// Each conforms to `SuggestionProvider`, so adding a source later (bookmarks,
/// commands…) is just another value in the provider list.
///
/// Ranking mirrors mainstream omnibox behaviour (Chrome/Safari/Firefox): typed
/// URLs and open-tab switches outrank history; history and top sites beat live
/// search suggestions; the fallback "Search for …" row anchors the bottom.

// ─── Scoring tiers ───────────────────────────────────────────────────────

/// Score bases for unified ranking in `SuggestionController`. Gaps are wide
/// enough that match quality within a tier only reorders inside it.
enum SuggestionScore {
    static let typedURL = 10_000
    static let openTabPrefix = 9_500
    static let openTabMatch = 8_800
    static let historyBase = 8_000
    static let topSitePrefix = 7_200
    static let topSite = 6_000
    static let searchSuggestBase = 600
    static let searchQuery = 50
}

// ─── Fuzzy match (core FFI) ──────────────────────────────────────────────

/// The omnibox fuzzy scorer from the Rust core — same algorithm history uses.
enum FuzzyMatch {
    struct Result {
        let score: Int
        let indices: [Int]
    }

    static func match(query: String, text: String) -> Result? {
        guard !query.isEmpty else { return nil }
        return query.withCString { qPtr in
            text.withCString { tPtr -> Result? in
                guard let raw = browser_fuzzy_match(qPtr, tPtr) else { return nil }
                defer { browser_string_free(raw) }
                let json = String(cString: raw)
                guard json != "null",
                      let data = json.data(using: .utf8),
                      let row = try? JSONDecoder().decode(Row.self, from: data)
                else {
                    return nil
                }
                return Result(score: row.score, indices: row.indices)
            }
        }
    }

    private struct Row: Decodable {
        let score: Int
        let indices: [Int]
    }
}

// ─── History ─────────────────────────────────────────────────────────────

/// Frecency-ranked pages the user has visited, from the Rust store via
/// `browser_query_suggestions`. Favicons come from the same store (captured
/// while browsing), resolved by `FaviconLoader`.
struct HistoryProvider: SuggestionProvider {
    func suggestions(for query: String) async -> [Suggestion] {
        guard let data = queryStore(query),
              let rows = try? JSONDecoder().decode([Row].self, from: data)
        else {
            return []
        }
        return rows.enumerated().map { index, row in
            let url = row.url
            let hasTitle = !row.title.isEmpty
            return Suggestion(
                id: "history:\(url)",
                icon: .favicon(host: row.host),
                title: hasTitle ? row.title : row.displayUrl,
                subtitle: hasTitle ? row.displayUrl : nil,
                run: { Navigation.open(url) },
                score: SuggestionScore.historyBase - index * 10,
                titleMatches: hasTitle ? row.titleMatches : row.urlMatches,
                subtitleMatches: hasTitle ? row.urlMatches : [],
                displayURL: row.displayUrl,
                dedupeKey: row.displayUrl.lowercased()
            )
        }
    }

    private func queryStore(_ query: String) -> Data? {
        query.withCString { ptr -> Data? in
            guard let raw = browser_query_suggestions(ptr) else { return nil }
            defer { browser_string_free(raw) }
            return Data(String(cString: raw).utf8)
        }
    }

    private struct Row: Decodable {
        let url: String
        let title: String
        let host: String
        let displayUrl: String
        let titleMatches: [Int]
        let urlMatches: [Int]
    }
}

// ─── Open tabs ─────────────────────────────────────────────────────────────

/// Open tabs in the current profile — "Switch to this tab" rows, boosted when
/// the query matches the tab's title or URL (Chrome's open-tab provider).
struct OpenTabsProvider: SuggestionProvider {
    func suggestions(for query: String) async -> [Suggestion] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let tabs = await MainActor.run {
            BrowserState.shared.tabs.filter { tab in
                tab.url.hasPrefix("http")
            }
        }
        guard !tabs.isEmpty else { return [] }

        // Zero-state: skip — an empty omnibox should surface frecency + top
        // sites, not every open tab.
        if q.isEmpty { return [] }

        var scored: [(Int, Suggestion)] = []
        for tab in tabs {
            guard let match = scoreTab(tab, query: q) else { continue }
            let display = prettyURL(tab.url)
            let hasTitle = !tab.title.isEmpty && tab.title != tab.host
            scored.append((
                match.score,
                Suggestion(
                    id: "tab:\(tab.id)",
                    icon: .favicon(host: tab.host),
                    title: hasTitle ? tab.title : display,
                    subtitle: hasTitle ? display : "Switch to Tab",
                    run: { BrowserState.shared.selectTab(tab.id) },
                    score: match.score,
                    titleMatches: match.titleMatches,
                    subtitleMatches: match.subtitleMatches,
                    displayURL: display,
                    dedupeKey: display.lowercased()
                )
            ))
        }
        return scored.sorted { $0.0 > $1.0 }.map(\.1)
    }

    private struct TabMatch {
        let score: Int
        let titleMatches: [Int]
        let subtitleMatches: [Int]
    }

    private func scoreTab(_ tab: TabItem, query: String) -> TabMatch? {
        let display = prettyURL(tab.url)
        let host = tab.host.lowercased()
        let normalized = normalizeQuery(query)

        // Host / URL prefix — inline-autocomplete quality, Chrome's tab boost.
        if !normalized.isEmpty, display.lowercased().hasPrefix(normalized) {
            let detail = FuzzyMatch.match(query: query, text: display)?.score ?? 0
            return TabMatch(
                score: SuggestionScore.openTabPrefix + detail,
                titleMatches: [],
                subtitleMatches: FuzzyMatch.match(query: query, text: display)?.indices ?? []
            )
        }
        if !normalized.isEmpty, host.hasPrefix(normalized) {
            let detail = FuzzyMatch.match(query: query, text: host)?.score ?? 0
            return TabMatch(
                score: SuggestionScore.openTabPrefix + detail,
                titleMatches: [],
                subtitleMatches: FuzzyMatch.match(query: query, text: host)?.indices ?? []
            )
        }

        // Fuzzy title / URL / host — pick the best field.
        let titleM = tab.title.isEmpty
            ? nil
            : FuzzyMatch.match(query: query, text: tab.title)
        let urlM = FuzzyMatch.match(query: query, text: display)
        let hostM = FuzzyMatch.match(query: query, text: host)

        let best: (Int, [Int], [Int])? = [
            titleM.map { (SuggestionScore.openTabMatch + $0.score, $0.indices, []) },
            urlM.map { (SuggestionScore.openTabMatch + $0.score, [], $0.indices) },
            hostM.map { (SuggestionScore.openTabMatch + $0.score, [], $0.indices) },
        ].compactMap { $0 }.max(by: { $0.0 < $1.0 })

        guard let best else { return nil }
        return TabMatch(score: best.0, titleMatches: best.1, subtitleMatches: best.2)
    }
}

// ─── Top sites ─────────────────────────────────────────────────────────────

/// Curated popular destinations for the zero-state omnibox and prefix matches
/// before the user has much history (Chrome's top-sites / Firefox's highlights).
struct TopSitesProvider: SuggestionProvider {
    private struct Site {
        let title: String
        let url: String
        var host: String { URLComponents(string: url)?.host ?? "" }
        var displayURL: String { prettyURL(url) }
    }

    /// A compact set of globally popular sites. Kept in the native layer as
    /// bootstrap data — user-specific frecency comes from `HistoryProvider`.
    private static let sites: [Site] = [
        Site(title: "Google", url: "https://www.google.com"),
        Site(title: "YouTube", url: "https://www.youtube.com"),
        Site(title: "GitHub", url: "https://github.com"),
        Site(title: "Wikipedia", url: "https://www.wikipedia.org"),
        Site(title: "Reddit", url: "https://www.reddit.com"),
        Site(title: "X", url: "https://x.com"),
        Site(title: "Amazon", url: "https://www.amazon.com"),
        Site(title: "Netflix", url: "https://www.netflix.com"),
        Site(title: "LinkedIn", url: "https://www.linkedin.com"),
        Site(title: "Stack Overflow", url: "https://stackoverflow.com"),
        Site(title: "Hacker News", url: "https://news.ycombinator.com"),
        Site(title: "Gmail", url: "https://mail.google.com"),
        Site(title: "Google Drive", url: "https://drive.google.com"),
        Site(title: "Twitch", url: "https://www.twitch.tv"),
        Site(title: "Spotify", url: "https://open.spotify.com"),
    ]

    func suggestions(for query: String) async -> [Suggestion] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        if q.isEmpty {
            return Self.sites.enumerated().map { index, site in
                makeSuggestion(site, score: SuggestionScore.topSite - index, matches: ([], []))
            }
        }
        let normalized = normalizeQuery(q)
        var scored: [(Int, Site, ([Int], [Int]))] = []
        for site in Self.sites {
            let display = site.displayURL
            let host = site.host.lowercased()
            let isPrefix = !normalized.isEmpty && (
                display.lowercased().hasPrefix(normalized) || host.hasPrefix(normalized)
            )
            let titleM = FuzzyMatch.match(query: q, text: site.title)
            let urlM = FuzzyMatch.match(query: q, text: display)
            let hostM = FuzzyMatch.match(query: q, text: host)

            let base: Int
            let matches: ([Int], [Int])
            if isPrefix {
                let detail = urlM?.score ?? hostM?.score ?? 0
                base = SuggestionScore.topSitePrefix + detail
                matches = ([], urlM?.indices ?? hostM?.indices ?? [])
            } else if let m = titleM {
                base = SuggestionScore.topSite + m.score
                matches = (m.indices, [])
            } else if let m = urlM {
                base = SuggestionScore.topSite + m.score
                matches = ([], m.indices)
            } else if let m = hostM {
                base = SuggestionScore.topSite + m.score
                matches = ([], m.indices)
            } else {
                continue
            }
            scored.append((base, site, matches))
        }
        return scored
            .sorted { $0.0 > $1.0 }
            .map { makeSuggestion($0.1, score: $0.0, matches: $0.2) }
    }

    private func makeSuggestion(
        _ site: Site,
        score: Int,
        matches: ([Int], [Int])
    ) -> Suggestion {
        Suggestion(
            id: "topsite:\(site.url)",
            icon: .favicon(host: site.host),
            title: site.title,
            subtitle: site.displayURL,
            run: { Navigation.open(site.url) },
            score: score,
            titleMatches: matches.0,
            subtitleMatches: matches.1,
            displayURL: site.displayURL,
            dedupeKey: site.displayURL.lowercased()
        )
    }
}

// ─── Search suggestions ──────────────────────────────────────────────────

/// Builds a search-results URL via the core-owned engine preference (ADR-0010).
enum SearchNavigation {
    static func url(for query: String) -> String {
        query.withCString { ptr -> String in
            guard let raw = browser_search_url_for_query(ptr) else {
                return "https://www.google.com/search?q=\(query)"
            }
            defer { browser_string_free(raw) }
            return String(cString: raw)
        }
    }
}

/// Live autocomplete endpoint for the selected search engine. Engines without a
/// public suggest API return nil and contribute no remote rows.
protocol SearchSuggestBackend: Sendable {
    func suggestionsURL(for query: String) -> URL?
    func parseSuggestions(_ data: Data) -> [String]
}

enum SearchSuggestBackends {
    static func current() -> (any SearchSuggestBackend)? {
        switch browser_settings_search_engine() {
        case 1: return DuckDuckGoSuggestBackend()
        case 2: return BingSuggestBackend()
        case 3: return BraveSuggestBackend()
        case 5: return EcosiaSuggestBackend()
        case 6: return KagiSuggestBackend()
        default: return GoogleSuggestBackend()
        }
    }
}

private struct GoogleSuggestBackend: SearchSuggestBackend {
    func suggestionsURL(for query: String) -> URL? {
        guard let q = query.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) else {
            return nil
        }
        return URL(string: "https://suggestqueries.google.com/complete/search?client=firefox&q=\(q)")
    }

    func parseSuggestions(_ data: Data) -> [String] {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [Any],
              root.count > 1,
              let completions = root[1] as? [String]
        else {
            return []
        }
        return completions
    }
}

private struct DuckDuckGoSuggestBackend: SearchSuggestBackend {
    func suggestionsURL(for query: String) -> URL? {
        guard let q = query.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) else {
            return nil
        }
        return URL(string: "https://duckduckgo.com/ac/?q=\(q)&type=list")
    }

    func parseSuggestions(_ data: Data) -> [String] {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [Any],
              root.count > 1,
              let completions = root[1] as? [String]
        else {
            return []
        }
        return completions
    }
}

private struct BingSuggestBackend: SearchSuggestBackend {
    func suggestionsURL(for query: String) -> URL? {
        guard let q = query.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) else {
            return nil
        }
        return URL(string: "https://api.bing.com/osjson.aspx?query=\(q)")
    }

    func parseSuggestions(_ data: Data) -> [String] {
        guard let completions = try? JSONSerialization.jsonObject(with: data) as? [String] else {
            return []
        }
        return completions
    }
}

private struct BraveSuggestBackend: SearchSuggestBackend {
    func suggestionsURL(for query: String) -> URL? {
        guard let q = query.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) else {
            return nil
        }
        return URL(string: "https://search.brave.com/api/suggest?q=\(q)")
    }

    func parseSuggestions(_ data: Data) -> [String] {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [Any],
              root.count > 1,
              let completions = root[1] as? [String]
        else {
            return []
        }
        return completions
    }
}

private struct EcosiaSuggestBackend: SearchSuggestBackend {
    func suggestionsURL(for query: String) -> URL? {
        guard let q = query.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) else {
            return nil
        }
        return URL(string: "https://ac.ecosia.org/autocomplete?q=\(q)")
    }

    func parseSuggestions(_ data: Data) -> [String] {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else {
            return []
        }
        return root.compactMap { $0["title"] as? String }
    }
}

private struct KagiSuggestBackend: SearchSuggestBackend {
    func suggestionsURL(for query: String) -> URL? {
        guard let q = query.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) else {
            return nil
        }
        return URL(string: "https://kagi.com/api/autosuggest?q=\(q)")
    }

    func parseSuggestions(_ data: Data) -> [String] {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [Any],
              root.count > 1,
              let completions = root[1] as? [String]
        else {
            return []
        }
        return completions
    }
}

/// Live query completions from the selected search engine.
struct SearchSuggestProvider: SuggestionProvider {
    var limit = 5

    func suggestions(for query: String) async -> [Suggestion] {
        guard let backend = SearchSuggestBackends.current(),
              let url = backend.suggestionsURL(for: query),
              let (data, _) = try? await URLSession.shared.data(from: url)
        else {
            return []
        }
        return backend.parseSuggestions(data).prefix(limit).enumerated().map { index, text in
            Suggestion(
                id: "search:\(text)",
                icon: .system("magnifyingglass"),
                title: text,
                subtitle: nil,
                run: { Navigation.open(SearchNavigation.url(for: text)) },
                score: SuggestionScore.searchSuggestBase - index * 10
            )
        }
    }
}

// ─── Search fallback ─────────────────────────────────────────────────────

/// The omnibox's bottom anchor row — "Search … for …" — always available when
/// the query is not already a URL.
struct SearchQueryProvider: SuggestionProvider {
    func suggestions(for query: String) async -> [Suggestion] {
        let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty, urlLikeHost(q) == nil else { return [] }
        let label = SearchEngineOption.current.label
        return [
            Suggestion(
                id: "search-query:\(q)",
                icon: .system("magnifyingglass"),
                title: "Search \(label) for \"\(q)\"",
                subtitle: nil,
                run: { Navigation.open(SearchNavigation.url(for: q)) },
                score: SuggestionScore.searchQuery
            ),
        ]
    }
}

// ─── Typed URL ───────────────────────────────────────────────────────────

/// When the query itself looks like a URL or bare host, offer a "go straight
/// there" row above everything else.
struct URLProvider: SuggestionProvider {
    func suggestions(for query: String) async -> [Suggestion] {
        guard let host = urlLikeHost(query) else { return [] }
        let target = query.contains("://") ? query : "https://\(query)"
        let display = prettyURL(target)
        return [
            Suggestion(
                id: "url:\(target)",
                icon: .favicon(host: host),
                title: query,
                subtitle: "Open site",
                run: { Navigation.open(target) },
                score: SuggestionScore.typedURL,
                displayURL: display,
                dedupeKey: display.lowercased()
            ),
        ]
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────

/// Strip scheme / `www.` from a typed query so `https://git` prefix-matches
/// `github.com` the same way the Rust history matcher does.
func normalizeQuery(_ query: String) -> String {
    var s = query.lowercased()
    for scheme in ["https://", "http://"] where s.hasPrefix(scheme) {
        s.removeFirst(scheme.count)
        break
    }
    if s.hasPrefix("www.") { s.removeFirst(4) }
    return s
}

/// The host of a URL-like query, or nil if it does not read as one.
func urlLikeHost(_ query: String) -> String? {
    let q = query.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !q.isEmpty, !q.contains(" ") else { return nil }
    guard q.contains("://") || q.contains(".") else { return nil }

    var rest = q
    if let schemeRange = rest.range(of: "://") {
        rest = String(rest[schemeRange.upperBound...])
    }
    let authority = rest.prefix { $0 != "/" && $0 != "?" && $0 != "#" }
    let host = authority.split(separator: "@").last.map(String.init) ?? String(authority)
    let bare = host.split(separator: ":").first.map(String.init) ?? host
    return bare.isEmpty ? nil : bare.lowercased()
}

/// A tidy display form of a URL for a suggestion subtitle.
func prettyURL(_ url: String) -> String {
    var s = url
    for scheme in ["https://", "http://"] where s.hasPrefix(scheme) {
        s.removeFirst(scheme.count)
    }
    if s.hasPrefix("www.") { s.removeFirst(4) }
    if s.hasSuffix("/") { s.removeLast() }
    return s
}

/// Unfocused address-bar label: the host only (`www.` stripped), matching Safari
/// and Chrome. Falls back to [`prettyURL`] when there is no host (`file:`, `data:`,
/// …); the full URL is shown once the field is focused for editing.
func displayDomain(_ url: String) -> String {
    guard let host = URLComponents(string: url)?.host, !host.isEmpty else {
        return prettyURL(url)
    }
    if host.hasPrefix("www.") { return String(host.dropFirst(4)) }
    return host
}

/// The standard omnibox provider set shared by the command bar and sidebar URL
/// field.
enum OmniboxProviders {
    static let local: [any SuggestionProvider] = [
        URLProvider(),
        OpenTabsProvider(),
        HistoryProvider(),
        TopSitesProvider(),
        SearchQueryProvider(),
    ]
    static let remote: [any SuggestionProvider] = [SearchSuggestProvider()]
}
