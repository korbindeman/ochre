import Foundation

/// The concrete suggestion sources plugged into the command bar's autocomplete.
/// Each conforms to `SuggestionProvider`, so adding a source later (open tabs,
/// bookmarks, commands…) is just another value in the provider list.

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
        return rows.map { row in
            let url = row.url
            // With a page title, show it up top and the URL as the subtitle;
            // without one, promote the URL to the title. The highlight ranges
            // follow whichever field each string came from.
            let hasTitle = !row.title.isEmpty
            return Suggestion(
                id: "history:\(url)",
                icon: .favicon(host: row.host),
                title: hasTitle ? row.title : row.displayUrl,
                subtitle: hasTitle ? row.displayUrl : nil,
                run: { Navigation.open(url) },
                titleMatches: hasTitle ? row.titleMatches : row.urlMatches,
                subtitleMatches: hasTitle ? row.urlMatches : [],
                displayURL: row.displayUrl
            )
        }
    }

    /// The JSON payload from the store, with its C string freed.
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

// ─── Search suggestions ──────────────────────────────────────────────────

/// A search engine's suggestion + query endpoints. Abstracted so the search
/// source is swappable while defaulting to Google.
protocol SearchEngine: Sendable {
    /// Endpoint whose response is a list of completions for `query`.
    func suggestionsURL(for query: String) -> URL?
    /// Parse that response into completion strings.
    func parseSuggestions(_ data: Data) -> [String]
    /// The page to open when searching `query`.
    func searchURL(for query: String) -> String
}

/// Google's suggest + search endpoints (the default engine).
struct GoogleSuggestEngine: SearchEngine {
    func suggestionsURL(for query: String) -> URL? {
        guard let q = query.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) else {
            return nil
        }
        return URL(string: "https://suggestqueries.google.com/complete/search?client=firefox&q=\(q)")
    }

    /// Google returns `["query", ["s1", "s2", …], …]`; the completions are the
    /// second element.
    func parseSuggestions(_ data: Data) -> [String] {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [Any],
              root.count > 1,
              let completions = root[1] as? [String]
        else {
            return []
        }
        return completions
    }

    func searchURL(for query: String) -> String {
        let q = query.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? query
        return "https://www.google.com/search?q=\(q)"
    }
}

/// Live query completions from a `SearchEngine` (Google by default). Selecting
/// one runs that engine's search.
struct SearchSuggestProvider: SuggestionProvider {
    var engine: SearchEngine = GoogleSuggestEngine()
    var limit = 5

    func suggestions(for query: String) async -> [Suggestion] {
        guard let url = engine.suggestionsURL(for: query),
              let (data, _) = try? await URLSession.shared.data(from: url)
        else {
            return []
        }
        let engine = engine
        return engine.parseSuggestions(data).prefix(limit).map { text in
            Suggestion(
                id: "search:\(text)",
                icon: .system("magnifyingglass"),
                title: text,
                subtitle: nil,
                run: { Navigation.open(engine.searchURL(for: text)) }
            )
        }
    }
}

// ─── Typed URL ───────────────────────────────────────────────────────────

/// When the query itself looks like a URL or bare host, offer a "go straight
/// there" row (with that site's favicon) above the search suggestions.
struct URLProvider: SuggestionProvider {
    func suggestions(for query: String) async -> [Suggestion] {
        guard let host = urlLikeHost(query) else { return [] }
        let target = query.contains("://") ? query : "https://\(query)"
        return [
            Suggestion(
                id: "url:\(target)",
                icon: .favicon(host: host),
                title: query,
                subtitle: "Open site",
                run: { Navigation.open(target) }
            )
        ]
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────

/// The host of a URL-like query, or nil if it does not read as one — a single
/// token (no spaces) with a scheme or an embedded dot. Mirrors the command
/// bar's own URL-vs-search rule.
private func urlLikeHost(_ query: String) -> String? {
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

/// A tidy display form of a URL for a suggestion subtitle: scheme and any
/// trailing slash dropped (`https://example.com/` → `example.com`).
func prettyURL(_ url: String) -> String {
    var s = url
    for scheme in ["https://", "http://"] where s.hasPrefix(scheme) {
        s.removeFirst(scheme.count)
    }
    if s.hasSuffix("/") { s.removeLast() }
    return s
}
