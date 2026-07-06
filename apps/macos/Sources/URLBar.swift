import AppKit
import SwiftUI

/// The sidebar's current-tab address bar. Where the original chip was a pure
/// *display* that routed clicks into the center command bar (ADR-0004), this one
/// edits **inline** in the sidebar (ADR-0014): clicking it — or ⌘L — turns the
/// chip into a live search/URL field whose autocomplete drops down within the
/// sidebar column, matching Arc. The command bar stays the ⌘T new-tab surface.
///
/// Editing state and the shared autocomplete engine live in `URLBarModel` so one
/// source of truth drives both sidebar instances (pinned + floating) and the ⌘L
/// menu path. Only the *visible* sidebar hosts the live field (`isVisible`), so
/// an off-screen instance can never focus a hidden field and steal keystrokes.

// ─── Model ───────────────────────────────────────────────────────────────────

@MainActor
final class URLBarModel: ObservableObject {
    static let shared = URLBarModel()

    @Published private(set) var isEditing = false

    /// Same providers as the command bar: instant local rows (typed URL, history)
    /// drive inline autocomplete; debounced Google suggestions merge in below.
    let suggestions = SuggestionController(
        local: [URLProvider(), HistoryProvider()],
        remote: [SearchSuggestProvider()]
    )

    /// The active tab's URL, captured when editing begins to seed the field.
    private(set) var initialQuery = ""

    /// Begin editing the active tab's address in place. Routes commits (the typed
    /// query and any chosen suggestion) to the current tab. No-op without a tab —
    /// the empty chip is a new-tab launcher instead.
    func beginEditing() {
        guard !isEditing, let tab = BrowserState.shared.activeTab else { return }
        Navigation.route = .currentTab
        initialQuery = tab.url
        suggestions.reset()
        if !initialQuery.isEmpty { suggestions.prime(query: initialQuery) }
        isEditing = true
    }

    /// Leave editing and hand keyboard focus back to the page. Mirrors the
    /// command bar's dismissal: the field resigned first responder as it left the
    /// hierarchy, so drive CEF `set_focus` to land keystrokes on the page again.
    func endEditing() {
        guard isEditing else { return }
        isEditing = false
        Navigation.route = .newTab
        suggestions.reset()
        browser_focus_active_tab()
    }

    /// Commit the typed query (Return with nothing highlighted).
    func commit() {
        Navigation.open(suggestions.currentQuery)
        endEditing()
    }

    /// Run a chosen suggestion (a row click, or Return on a highlighted row).
    func choose(_ suggestion: Suggestion) {
        suggestion.run()
        endEditing()
    }
}

// ─── View ────────────────────────────────────────────────────────────────────

struct URLBar: View {
    /// True only for the on-screen sidebar instance; the off-screen one keeps
    /// showing the static chip so it never focuses a hidden field.
    let isVisible: Bool

    @ObservedObject private var state = BrowserState.shared
    @ObservedObject private var model = URLBarModel.shared
    @ObservedObject private var suggestions = URLBarModel.shared.suggestions
    @State private var hovering = false
    @State private var copied = false

    /// Fixed height, shared by the chip and the editor so the swap doesn't jump,
    /// and used to drop the results list just below the field.
    private let fieldHeight: CGFloat = 32

    private var tab: TabItem? { state.activeTab }
    private var isEmpty: Bool { tab?.url.isEmpty ?? true }
    private var text: String {
        guard let tab, !tab.url.isEmpty else { return "Search or enter URL" }
        return prettyURL(tab.url)
    }

    var body: some View {
        if model.isEditing && isVisible {
            editor
        } else {
            chip
        }
    }

    // ─── Display chip ──────────────────────────────────────────────────────────

    private var chip: some View {
        // The whole chip taps to edit (⌘L); the hover-revealed copy button is a
        // nested action, so — like TabRow's ✕ — the row uses `onTapGesture`
        // rather than a Button to keep the inner button's tap unambiguous.
        HStack(spacing: 6) {
            Image(systemName: isEmpty ? "magnifyingglass" : "globe")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.secondary)
            Text(text)
                .lineLimit(1)
                .truncationMode(.tail)
                .foregroundStyle(isEmpty ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
            Spacer(minLength: 0)
            if hovering, let url = tab?.url, !url.isEmpty {
                Button(action: { copyLink(url) }) {
                    Image(systemName: copied ? "checkmark" : "link")
                        .font(.system(size: 11, weight: .medium))
                        .frame(width: 18, height: 18)
                        .contentShape(.rect(cornerRadius: 5))
                }
                .buttonStyle(.plain)
                .foregroundStyle(copied ? AnyShapeStyle(.green) : AnyShapeStyle(.secondary))
                .help("Copy Link")
            }
        }
        .font(.system(size: 13))
        .padding(.horizontal, 10)
        .frame(height: fieldHeight)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(.quaternary)
                .opacity(hovering ? 0.7 : 0.4)
        )
        .contentShape(.rect(cornerRadius: 8))
        .onTapGesture(perform: activate)
        .onHover { hovering = $0 }
        .help(isEmpty ? "New Tab (⌘T)" : "Edit URL (⌘L)")
    }

    private func activate() {
        if isEmpty {
            (NSApp.delegate as? AppDelegate)?.toggleCommandBar()
        } else {
            model.beginEditing()
        }
    }

    /// Copy the active tab's full URL, mirroring the history row's Copy Link.
    /// A brief checkmark confirms the copy landed.
    private func copyLink(_ url: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(url, forType: .string)
        copied = true
        Task {
            try? await Task.sleep(for: .seconds(1))
            copied = false
        }
    }

    // ─── Inline editor ─────────────────────────────────────────────────────────

    private var editor: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.secondary)
            AutocompleteField(
                controller: suggestions,
                initialQuery: model.initialQuery,
                font: .system(size: 13),
                onFocusChange: { focused in
                    // Clicking into the page (or elsewhere) resigns the field —
                    // dismiss, matching the command bar losing key. Suggestion
                    // taps don't take first responder, so they never trip this.
                    if !focused { model.endEditing() }
                }
            )
        }
        .padding(.horizontal, 10)
        .frame(height: fieldHeight)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(Color(nsColor: .textBackgroundColor))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .strokeBorder(Color.accentColor, lineWidth: 2)
        )
        .onKeyPress(.upArrow) { suggestions.moveSelection(by: -1); return .handled }
        .onKeyPress(.downArrow) { suggestions.moveSelection(by: 1); return .handled }
        .onExitCommand { model.endEditing() }
        .onSubmit {
            if let selected = suggestions.selected() {
                model.choose(selected)
            } else {
                model.commit()
            }
        }
        // The results drop below the field as a floating card over the tab list
        // (the sidebar lifts this whole bar with `.zIndex`), so the list never
        // reflows as suggestions come and go.
        .overlay(alignment: .top) {
            if !suggestions.suggestions.isEmpty {
                AutocompleteList(controller: suggestions, onChoose: { model.choose($0) })
                    .padding(6)
                    .background(
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .fill(Color(nsColor: .windowBackgroundColor))
                            .shadow(color: .black.opacity(0.18), radius: 12, y: 4)
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1)
                    )
                    .fixedSize(horizontal: false, vertical: true)
                    .offset(y: fieldHeight + 6)
            }
        }
    }
}
