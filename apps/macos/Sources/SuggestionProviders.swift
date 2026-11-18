import Foundation

/// The concrete suggestion sources plugged into the command bar's autocomplete.
/// Each conforms to `SuggestionProvider`, so adding a source later (bookmarks,
/// commands…) is just another value in the provider list.
///
/// Page, tab and search ranking lives in the core (`OmniboxCore`, ADR-0071).
/// The providers here are what only the host knows: live search suggestions
/// (fetched here, ranked by the core) and browser commands.

// ─── Scoring tiers ───────────────────────────────────────────────────────

/// Ordering among host rows (commands); the core ranks everything else.
enum SuggestionScore {
    static let command = 8_700
}

// ─── Fuzzy match (core FFI) ──────────────────────────────────────────────

/// The omnibox fuzzy scorer from the Rust core - same algorithm history uses.
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
                score: -index
            )
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────

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
    static let local: [any SuggestionProvider] = []
    static let remote: [any SuggestionProvider] = [SearchSuggestProvider()]
}

/// Cmd-T adds browser actions and Space switching to the regular omnibox
/// sources. The inline sidebar address bar stays navigation-only.
enum CommandBarProviders {
    static let local: [any SuggestionProvider] = [
        BrowserCommandsProvider(),
        SpaceCommandsProvider(),
    ]
    static let remote = OmniboxProviders.remote
}
