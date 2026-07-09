import AppKit
import SwiftUI

/// A reusable, source-agnostic autocomplete: a suggestion model, a provider
/// protocol, a controller that fans a query out across providers, and a list
/// view that renders the results with favicons and keyboard selection.
///
/// It is deliberately decoupled from the command bar so the *same* autocomplete
/// can back other surfaces later (a command palette, a tab switcher, a link
/// picker). A host supplies its own text field and hands this component:
///   1. an ordered set of `SuggestionProvider`s (what to suggest), and
///   2. a query string (updated as the user types).
/// The host then renders `AutocompleteList` and forwards arrow/return/escape
/// keys via `autocompleteKeyboard(_:onSubmit:onCancel:)`.

// ─── Model ───────────────────────────────────────────────────────────────

/// One row in the dropdown. `run` performs the row's action (navigate, search,
/// run a command…); the provider that produced the suggestion owns that meaning,
/// which is what keeps the list view generic.
struct Suggestion: Identifiable, Sendable {
    let id: String
    let icon: SuggestionIcon
    let title: String
    let subtitle: String?
    let run: @MainActor @Sendable () -> Void
    /// Unified ranking score — higher rows float up after providers merge.
    var score: Int = 0
    /// Character offsets in `title` / `subtitle` the query matched, for
    /// highlighting. Empty when the source has nothing to emphasise (a live
    /// search completion, say).
    var titleMatches: [Int] = []
    var subtitleMatches: [Int] = []
    /// The normalized URL (scheme + `www.` stripped) this row points at, when it
    /// is a page — the basis for inline autocomplete. `nil` for non-page rows
    /// (a web search).
    var displayURL: String? = nil
    /// Key for cross-source deduplication (usually a normalized page URL).
    var dedupeKey: String? = nil
}

/// How to draw a suggestion's leading icon.
enum SuggestionIcon: Sendable {
    /// A site favicon, resolved by `FaviconLoader` (captured → web fallback → globe).
    case favicon(host: String)
    /// An SF Symbol (e.g. "magnifyingglass" for a search row).
    case system(String)
}

/// A source of suggestions. Each provider is one "kind" of result; the
/// controller merges several. `Sendable` so providers can run concurrently.
protocol SuggestionProvider: Sendable {
    func suggestions(for query: String) async -> [Suggestion]
}

// ─── Controller ────────────────────────────────────────────────────────────

/// Fans the query out across providers and publishes the merged list, the
/// keyboard selection, and an inline-autocomplete completion for the default
/// match.
///
/// Providers come in two tiers, mirroring a real omnibox: **local** ones (typed
/// URL, history) are cheap and run on every keystroke with no delay, so results
/// and the inline completion appear instantly; **remote** ones (live search
/// suggestions over the network) run after a short debounce and merge in below.
@MainActor
final class SuggestionController: ObservableObject {
    @Published private(set) var suggestions: [Suggestion] = []
    /// Index of the highlighted row, or nil when the list is empty.
    @Published var selection: Int?
    /// The field text the host should show for the default match, with the part
    /// beyond what the user typed offered as a selected completion (type "git",
    /// get "github.com"). Nil when there is nothing to complete. Only ever set
    /// on a forward edit, so it never fights the user deleting.
    @Published private(set) var inline: String?
    /// The most recent typed query fed to `update` (never an inline tail).
    private(set) var currentQuery = ""
    /// What Return should navigate to when nothing is arrow-highlighted: the
    /// inline completion if one is showing, otherwise the typed query.
    var commitQuery: String { inline ?? currentQuery }

    private let localProviders: [any SuggestionProvider]
    private let remoteProviders: [any SuggestionProvider]
    private let maxResults: Int
    private let debounce: Duration

    private var localTask: Task<Void, Never>?
    private var remoteTask: Task<Void, Never>?
    /// Local results held so a later remote batch can merge below them.
    private var localResults: [Suggestion] = []
    private var allowInline = false

    init(
        local: [any SuggestionProvider],
        remote: [any SuggestionProvider] = [],
        maxResults: Int = 8,
        debounce: Duration = .milliseconds(140)
    ) {
        self.localProviders = local
        self.remoteProviders = remote
        self.maxResults = maxResults
        self.debounce = debounce
    }

    /// Feed the latest query. `allowInline` is true only for forward typing, so
    /// the inline completion never re-appears while the user is deleting.
    /// Cancels any in-flight lookups. An empty query clears the list unless
    /// `zeroState` is true (explicit zero-state hosts only).
    func update(query rawQuery: String, allowInline: Bool, zeroState: Bool = false) {
        localTask?.cancel()
        remoteTask?.cancel()
        currentQuery = rawQuery
        self.allowInline = allowInline
        let query = rawQuery.trimmingCharacters(in: .whitespacesAndNewlines)

        guard !query.isEmpty || zeroState else {
            localResults = []
            suggestions = []
            selection = nil
            inline = nil
            return
        }

        // Local tier: instant. Publishes results + the inline completion.
        localTask = Task { [weak self] in
            guard let self else { return }
            let locals = await self.gather(self.localProviders, query)
            guard !Task.isCancelled else { return }
            self.publishLocal(locals)
        }

        // Remote tier: debounced. Skipped for an empty query — there is nothing
        // to complete and the zero-state is entirely local.
        guard !remoteProviders.isEmpty, !query.isEmpty else { return }
        remoteTask = Task { [weak self, debounce] in
            try? await Task.sleep(for: debounce)
            guard let self, !Task.isCancelled else { return }
            let remotes = await self.gather(self.remoteProviders, query)
            guard !Task.isCancelled else { return }
            self.publishRemote(remotes)
        }
    }

    /// Populate the zero-state dropdown (top sites, frecency history). No current
    /// host calls this — omniboxes stay collapsed until the user types.
    func showZeroState() {
        update(query: "", allowInline: false, zeroState: true)
    }

    /// Clear everything (e.g. when the host closes).
    func reset() {
        localTask?.cancel()
        remoteTask?.cancel()
        localResults = []
        suggestions = []
        selection = nil
        inline = nil
        currentQuery = ""
    }

    /// Seed the commit query without fetching suggestions. Used to prefill an
    /// edit-in-place session (⌘L): pressing Return with no edits then navigates
    /// to the shown URL, but the dropdown stays closed until the user types.
    func prime(query: String) {
        currentQuery = query
    }

    /// The keyboard-highlighted suggestion (arrow navigation), if any. Hover does
    /// not count — return commits the field text unless the user arrowed to a row.
    func selected() -> Suggestion? {
        guard let selection, suggestions.indices.contains(selection) else { return nil }
        return suggestions[selection]
    }

    /// Move the highlight by `delta` rows, clamped to the list bounds.
    func moveSelection(by delta: Int) {
        guard !suggestions.isEmpty else { return }
        let current = selection ?? (delta > 0 ? -1 : 0)
        selection = min(max(current + delta, 0), suggestions.count - 1)
    }

    // ─── Publishing ────────────────────────────────────────────────────────

    private func publishLocal(_ locals: [Suggestion]) {
        // Score-first merge across providers, then float the best host-prefix
        // match to the top (bare domains beat deep paths at equal tier).
        var ordered = locals.sorted { $0.score > $1.score }
        if let idx = bestHostPrefixIndex(in: ordered, typed: currentQuery), idx != 0 {
            ordered.insert(ordered.remove(at: idx), at: 0)
        }
        localResults = ordered
        suggestions = dedupe(ordered)
        selection = nil
        inline = allowInline ? bestPrefixFill(in: ordered, typed: currentQuery) : nil
    }

    private func publishRemote(_ remotes: [Suggestion]) {
        // Live search suggestions sit above the fallback "Search for …" row but
        // below page/tab/history matches.
        let (tail, head) = localResults.reduce(into: ([Suggestion](), [Suggestion]())) { acc, row in
            if row.score <= SuggestionScore.searchQuery {
                acc.0.append(row)
            } else {
                acc.1.append(row)
            }
        }
        let merged = head + remotes.sorted { $0.score > $1.score } + tail
        suggestions = dedupe(merged)
        // Keep the current highlight if still valid; otherwise clear it. Never
        // disturb the inline completion — remote rows never drive it.
        if let s = selection, !suggestions.indices.contains(s) {
            selection = nil
        }
    }

    /// The best inline completion across `list`, preferring a host-level prefix
    /// (type "you" → "youtube.com") over a deep-path prefix.
    private func bestPrefixFill(in list: [Suggestion], typed: String) -> String? {
        let hostMatches = list.compactMap { hostPrefixFill($0, typed) }
        if let shortest = hostMatches.min(by: { $0.count < $1.count }) {
            return shortest
        }
        return list.lazy.compactMap { self.displayPrefixFill($0, typed) }.first
    }

    /// Index of the best host-prefix row: shallowest URL first (bare domain over
    /// deep path), then highest score — so TopSites `youtube.com` beats a
    /// frecency-heavy `youtube.com/watch?v=…` from history.
    private func bestHostPrefixIndex(in list: [Suggestion], typed: String) -> Int? {
        list.enumerated()
            .filter { hostPrefixFill($0.element, typed) != nil }
            .min(by: {
                let d0 = displayPathDepth($0.element.displayURL ?? "")
                let d1 = displayPathDepth($1.element.displayURL ?? "")
                if d0 != d1 { return d0 < d1 }
                return $0.element.score > $1.element.score
            })?
            .offset
    }

    /// Path segments after the host — `youtube.com` is 0, `youtube.com/watch` is 1.
    private func displayPathDepth(_ displayURL: String) -> Int {
        max(0, displayURL.split(separator: "/").count - 1)
    }

    /// Complete to the bare host when the query is a host prefix — "you" becomes
    /// "youtube.com", not "youtube.com/watch?v=…".
    private func hostPrefixFill(_ suggestion: Suggestion, _ typed: String) -> String? {
        guard let display = suggestion.displayURL else { return nil }
        let q = typed.lowercased()
        guard !q.isEmpty, q == q.trimmingCharacters(in: .whitespaces) else { return nil }
        let host = display.split(separator: "/", maxSplits: 1).first.map(String.init) ?? display
        let lowerHost = host.lowercased()
        guard lowerHost.hasPrefix(q), host.count > typed.count else { return nil }
        return typed + host.dropFirst(typed.count)
    }

    /// Full display-URL prefix completion — used when no host-level match exists.
    private func displayPrefixFill(_ suggestion: Suggestion, _ typed: String) -> String? {
        guard let display = suggestion.displayURL else { return nil }
        let q = typed.lowercased()
        guard !q.isEmpty, q == q.trimmingCharacters(in: .whitespaces) else { return nil }
        let lowerDisplay = display.lowercased()
        guard lowerDisplay.hasPrefix(q), display.count > typed.count else { return nil }
        return typed + display.dropFirst(typed.count)
    }

    private func dedupe(_ list: [Suggestion]) -> [Suggestion] {
        var seen = Set<String>()
        return Array(
            list.filter { row in
                let key = row.dedupeKey ?? row.id
                return seen.insert(key).inserted
            }.prefix(maxResults))
    }

    private func gather(_ providers: [any SuggestionProvider], _ query: String) async
        -> [Suggestion]
    {
        // Run providers concurrently but keep results in provider order, so the
        // merged list is stable regardless of which source returns first.
        await withTaskGroup(of: (Int, [Suggestion]).self) { group in
            for (index, provider) in providers.enumerated() {
                group.addTask { (index, await provider.suggestions(for: query)) }
            }
            var byProvider = [[Suggestion]](repeating: [], count: providers.count)
            for await (index, chunk) in group {
                byProvider[index] = chunk
            }
            return byProvider.flatMap { $0 }
        }
    }
}

// ─── List view ─────────────────────────────────────────────────────────────

/// Renders the controller's suggestions. Purely presentational: keyboard
/// selection lives in the controller (arrows + return); hover highlight is
/// local so return still commits the field text. Clicking a row calls
/// `onChoose` (the host wires that to "run the suggestion, then dismiss"). Drop
/// it into any host below its text field.
struct AutocompleteList: View {
    @ObservedObject var controller: SuggestionController
    let onChoose: (Suggestion) -> Void
    /// Visual hover highlight only — does not affect return-key behaviour.
    @State private var hoverIndex: Int?

    var body: some View {
        VStack(spacing: 1) {
            ForEach(Array(controller.suggestions.enumerated()), id: \.element.id) {
                index, suggestion in
                SuggestionRow(
                    suggestion: suggestion,
                    isSelected: isHighlighted(index)
                )
                .contentShape(.rect)
                .onHover { hovering in
                    hoverIndex = hovering ? index : (hoverIndex == index ? nil : hoverIndex)
                }
                .onTapGesture { onChoose(suggestion) }
            }
        }
        .onChange(of: controller.suggestions.count) { _, _ in
            hoverIndex = nil
        }
    }

    private func isHighlighted(_ index: Int) -> Bool {
        if let selection = controller.selection { return selection == index }
        return hoverIndex == index
    }
}

/// Render `text` with the characters at `matches` (character offsets, as the
/// Rust matcher reports them) emphasised — heavier weight, and full-strength
/// colour when `dimUnmatched` fades the rest — so the user sees exactly what
/// their query hit. Offsets are treated as `Character` positions, which lines up
/// with the matcher's scalar indices for the URLs and titles it sees.
private func highlighted(
    _ text: String,
    matches: [Int],
    size: CGFloat,
    dimUnmatched: Bool
) -> AttributedString {
    let matched = Set(matches)
    var result = AttributedString()
    for (index, character) in text.enumerated() {
        var piece = AttributedString(String(character))
        if matched.contains(index) {
            piece.font = .system(size: size, weight: .semibold)
            piece.foregroundColor = .primary
        } else {
            piece.font = .system(size: size)
            if dimUnmatched { piece.foregroundColor = .secondary }
        }
        result += piece
    }
    return result
}

private struct SuggestionRow: View {
    let suggestion: Suggestion
    let isSelected: Bool

    var body: some View {
        HStack(spacing: 10) {
            SuggestionIconView(icon: suggestion.icon)
                .frame(width: 18, height: 18)

            Text(
                highlighted(
                    suggestion.title, matches: suggestion.titleMatches, size: 14,
                    dimUnmatched: false)
            )
            .lineLimit(1)

            if let subtitle = suggestion.subtitle, !subtitle.isEmpty {
                Text(
                    highlighted(
                        subtitle, matches: suggestion.subtitleMatches, size: 13, dimUnmatched: true)
                )
                .lineLimit(1)
                .layoutPriority(-1)
            }

            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(isSelected ? Color.primary.opacity(0.08) : .clear)
        )
        .contentShape(.rect)
    }
}

/// Draws a suggestion icon: an SF Symbol, or a favicon loaded asynchronously
/// (with a globe placeholder until it arrives / if there is none).
private struct SuggestionIconView: View {
    let icon: SuggestionIcon
    @State private var favicon: NSImage?

    var body: some View {
        switch icon {
        case .system(let name):
            Image(systemName: name)
                .font(.system(size: 14))
                .foregroundStyle(.secondary)
        case .favicon(let host):
            Group {
                if let favicon {
                    Image(nsImage: favicon).resizable().interpolation(.high)
                } else {
                    Image(systemName: "globe")
                        .font(.system(size: 14))
                        .foregroundStyle(.secondary)
                }
            }
            .task(id: host) { favicon = await FaviconLoader.shared.icon(for: host) }
        }
    }
}

// ─── Text field ────────────────────────────────────────────────────────────

/// A search/URL text field wired to a `SuggestionController`, carrying the
/// inline-autocomplete behaviour both the command bar and the sidebar URL bar
/// share: the default match is shown with its completed tail selected, and the
/// completion is suppressed while the user deletes. The host owns the field's
/// chrome, the results list, and keyboard navigation (arrows/return/escape) —
/// this view only manages the field text and feeds the controller.
struct AutocompleteField: View {
    @ObservedObject var controller: SuggestionController
    var placeholder = "Search or enter URL"
    var font: Font = .system(size: 18)
    /// Notified when the field gains or loses focus. The sidebar URL bar
    /// dismisses itself on focus loss; the command bar (a key panel) ignores it.
    var onFocusChange: (Bool) -> Void = { _ in }
    /// Select the entire field on focus and on click — standard address-bar UX.
    var selectAllOnFocus: Bool = false

    /// The text shown in the field, which may carry an inline-autocomplete tail
    /// beyond what the user typed.
    @State private var fieldText: String
    /// What the user has actually typed (the field minus any inline tail) — drives
    /// the query and the forward-vs-delete decision that gates inline completion.
    @State private var userText: String
    /// Set while we write an inline completion, so the resulting `onChange` isn't
    /// mistaken for the user typing.
    @State private var applyingInline = false
    @State private var selection: TextSelection?
    @FocusState private var focused: Bool

    init(
        controller: SuggestionController,
        initialQuery: String = "",
        placeholder: String = "Search or enter URL",
        font: Font = .system(size: 18),
        onFocusChange: @escaping (Bool) -> Void = { _ in },
        selectAllOnFocus: Bool = false
    ) {
        _controller = ObservedObject(wrappedValue: controller)
        self.placeholder = placeholder
        self.font = font
        self.onFocusChange = onFocusChange
        self.selectAllOnFocus = selectAllOnFocus
        _fieldText = State(initialValue: initialQuery)
        _userText = State(initialValue: initialQuery)
    }

    var body: some View {
        Group {
            if selectAllOnFocus {
                coreField.simultaneousGesture(TapGesture().onEnded { selectAllText() })
            } else {
                coreField
            }
        }
    }

    private var coreField: some View {
        TextField(placeholder, text: $fieldText, selection: $selection)
            .textFieldStyle(.plain)
            .font(font)
            .focused($focused)
            .onChange(of: fieldText) { _, newValue in
                // Ignore the field change caused by our own inline write.
                if applyingInline {
                    applyingInline = false
                    return
                }
                // Forward = the user typed (grew the text) or replaced a
                // selection; a plain backspace leaves `newValue` a prefix of the
                // old typed text, and must not re-trigger the completion it just
                // removed.
                let forward = newValue.count > userText.count || !userText.hasPrefix(newValue)
                userText = newValue
                controller.update(query: newValue, allowInline: forward)
            }
            .onReceive(controller.$inline) { applyInline($0) }
            .onChange(of: focused) { _, isFocused in
                onFocusChange(isFocused)
                if isFocused && selectAllOnFocus { selectAllText() }
            }
            .onAppear {
                focused = true
                if selectAllOnFocus {
                    selectAllText()
                } else if !fieldText.isEmpty {
                    // A prefilled field (edit-in-place) starts fully selected, so the
                    // first keystroke replaces it (matching ⌘L in Safari/Arc).
                    selection = TextSelection(range: fieldText.startIndex..<fieldText.endIndex)
                }
            }
    }

    private func selectAllText() {
        guard !fieldText.isEmpty else { return }
        selection = TextSelection(range: fieldText.startIndex..<fieldText.endIndex)
    }

    /// Show `fill` in the field with everything past what the user typed
    /// selected, so the next keystroke replaces just the completion (type "git",
    /// see "github.com" with "hub.com" selected). No-op when the completion no
    /// longer extends the current typed text.
    private func applyInline(_ fill: String?) {
        guard let fill,
            fill != fieldText,
            fill.count > userText.count,
            fill.lowercased().hasPrefix(userText.lowercased())
        else { return }
        applyingInline = true
        fieldText = fill
        if let tailStart = fill.index(
            fill.startIndex, offsetBy: userText.count, limitedBy: fill.endIndex)
        {
            selection = TextSelection(range: tailStart..<fill.endIndex)
        }
    }
}

// ─── Keyboard wiring ─────────────────────────────────────────────────────

extension View {
    /// SwiftUI-native keyboard routing for hosts that keep focus in an ordinary
    /// `TextField`: arrows move the keyboard selection, return chooses the
    /// arrow-highlighted suggestion (via `onChoose`) or calls `onSubmit` when
    /// nothing was arrowed to (hover does not count), escape calls `onCancel`.
    ///
    /// Note: hosts embedded in a borderless `NSPanel` (like the command bar) may
    /// prefer an AppKit key monitor, which intercepts arrow keys before the
    /// focused field more reliably than `onKeyPress` — see `CommandBarController`.
    func autocompleteKeyboard(
        _ controller: SuggestionController,
        onChoose: @escaping (Suggestion) -> Void,
        onSubmit: @escaping () -> Void,
        onCancel: @escaping () -> Void
    ) -> some View {
        self
            .onKeyPress(.upArrow) {
                controller.moveSelection(by: -1)
                return .handled
            }
            .onKeyPress(.downArrow) {
                controller.moveSelection(by: 1)
                return .handled
            }
            .onKeyPress(.return) {
                if let selected = controller.selected() {
                    onChoose(selected)
                } else {
                    onSubmit()
                }
                return .handled
            }
            .onKeyPress(.escape) {
                onCancel()
                return .handled
            }
    }
}
