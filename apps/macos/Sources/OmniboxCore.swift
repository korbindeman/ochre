import Foundation

/// The host side of the core omnibox ranker (ADR-0071): asks the core for the
/// ranked rows and turns them into `Suggestion`s. Running a row also teaches
/// the core what this input led to.
@MainActor
enum OmniboxCore {
    struct Result {
        let rows: [Suggestion]
        /// `rows[0]` is what Enter runs.
        let hasDefault: Bool
        let inline: String?
    }

    static func query(
        text: String, preventInline: Bool, remote: [String], previousDefault: String?,
        asyncPass: Bool
    ) -> Result {
        let request = Request(
            text: text, prevent_inline: preventInline, remote: remote,
            previous_default: previousDefault, async_pass: asyncPass)
        guard let body = try? JSONEncoder().encode(request),
            let json = String(data: body, encoding: .utf8),
            let raw = json.withCString({ browser_omnibox_query($0) })
        else { return Result(rows: [], hasDefault: false, inline: nil) }
        let data = Data(String(cString: raw).utf8)
        browser_string_free(raw)
        guard let reply = try? JSONDecoder().decode(Reply.self, from: data) else {
            return Result(rows: [], hasDefault: false, inline: nil)
        }
        let input = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let rows = reply.rows.enumerated().map { index, row in
            suggestion(row, input: input, isDefault: reply.has_default && index == 0)
        }
        return Result(rows: rows, hasDefault: reply.has_default, inline: reply.inline)
    }

    private static func suggestion(_ row: Row, input: String, isDefault: Bool) -> Suggestion {
        if row.kind == "search" {
            let query = row.query
            let verbatim = query.caseInsensitiveCompare(input) == .orderedSame
            return Suggestion(
                id: row.key,
                icon: .system("magnifyingglass"),
                title: verbatim
                    ? "Search \(SearchEngineOption.current.label) for \"\(query)\"" : query,
                subtitle: nil,
                run: { Navigation.open(SearchNavigation.url(for: query)) },
                score: row.relevance,
                dedupeKey: row.key
            )
        }
        let url = row.url
        let display = prettyURL(url)
        // Private windows never switch to (or reveal) a normal-window tab.
        let tab = BrowserState.current.isPrivate ? nil : row.tab
        let host = row.host ?? urlHost(url)
        return Suggestion(
            id: row.key,
            icon: .favicon(host: host),
            title: row.title,
            subtitle: tab != nil ? "Switch to Tab" : row.subtitle,
            run: {
                // The default URL row is what the user typed or accepted inline.
                browser_omnibox_note_choice(input, url, isDefault)
                if let tab {
                    BrowserState.normal.selectTab(tab)
                } else {
                    Navigation.open(url)
                }
            },
            score: row.relevance,
            titleMatches: row.title_matches.map(Int.init),
            subtitleMatches: tab != nil ? [] : row.subtitle_matches.map(Int.init),
            displayURL: display,
            dedupeKey: row.key
        )
    }

    private static func urlHost(_ url: String) -> String {
        URLComponents(string: url)?.host ?? display(url)
    }

    private static func display(_ url: String) -> String { prettyURL(url) }

    private struct Request: Encodable {
        let text: String
        let prevent_inline: Bool
        let remote: [String]
        let previous_default: String?
        let async_pass: Bool
    }

    private struct Row: Decodable {
        let key: String
        let kind: String
        let title: String
        let subtitle: String?
        let url: String
        let query: String
        let host: String?
        let tab: Int32?
        let title_matches: [UInt32]
        let subtitle_matches: [UInt32]
        let relevance: Int
    }

    private struct Reply: Decodable {
        let rows: [Row]
        let has_default: Bool
        let inline: String?
    }
}
