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
    /// Bumped whenever editing begins so the field remounts and selects all (⌘L).
    @Published private(set) var editingSession: UInt = 0

    /// Same providers as the command bar: instant local rows (typed URL, history)
    /// drive inline autocomplete; debounced Google suggestions merge in below.
    let suggestions = SuggestionController(
        local: OmniboxProviders.local,
        remote: OmniboxProviders.remote
    )

    /// The active tab's URL, captured when editing begins to seed the field.
    private(set) var initialQuery = ""

    /// Begin editing the active tab's address in place. Routes commits (the typed
    /// query and any chosen suggestion) to the current tab. No-op without a tab —
    /// the empty chip is a new-tab launcher instead.
    func beginEditing() {
        let state = BrowserState.shared
        guard !isEditing, !state.isSurfaceOpen, let tab = state.activeTab else { return }
        Navigation.route = .currentTab
        initialQuery = tab.url
        suggestions.reset()
        if !initialQuery.isEmpty { suggestions.prime(query: initialQuery) }
        editingSession &+= 1
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

// ─── Placement ───────────────────────────────────────────────────────────────

enum ViewportURLBarMetrics {
    /// Top chrome reserved above the web viewport (bar + breathing room).
    static let chromeInset: CGFloat = 64
    /// The floating address pill — large, capsule-shaped (Safari-style).
    static let pillHeight: CGFloat = 40
    static let pillMaxWidth: CGFloat = 560
    /// Alias for layout code that sizes the overlay host.
    static var height: CGFloat { chromeInset }
}

/// Shared Liquid Glass capsule for the in-page address pill and its flanking
/// controls — one material path so they read as the same height and weight.
struct ViewportGlassCapsule: View {
    var body: some View {
        if #available(macOS 26.0, *) {
            Capsule()
                .fill(.clear)
                .glassEffect(.regular, in: .capsule)
        } else {
            Capsule()
                .fill(.regularMaterial)
        }
    }
}

enum URLBarPlacement {
    /// Arc-style compact chip in the sidebar column.
    case sidebar
    /// Safari-style bar floating over the page viewport.
    case viewport
}

// ─── View ────────────────────────────────────────────────────────────────────

struct URLBar: View {
    let placement: URLBarPlacement
    /// True only for the on-screen sidebar instance; the off-screen one keeps
    /// showing the static chip so it never focuses a hidden field. Ignored for
    /// viewport placement (the overlay is always the live instance).
    var isVisible: Bool = true

    @ObservedObject private var state = BrowserState.shared
    @ObservedObject private var model = URLBarModel.shared
    @ObservedObject private var suggestions = URLBarModel.shared.suggestions
    @State private var hovering = false
    @State private var copied = false
    @State private var showShield = false
    @State private var showVault = false
    @State private var showViewportSuggestions = false

    /// Fixed height, shared by the chip and the editor so the swap doesn't jump,
    /// and used to drop the results list just below the field.
    private var fieldHeight: CGFloat {
        placement == .viewport ? ViewportURLBarMetrics.pillHeight : 32
    }

    private var tab: TabItem? { state.activeTab }
    private var hostsEditing: Bool {
        switch placement {
        case .sidebar: return model.isEditing && isVisible && !isNeutralized
        case .viewport: return model.isEditing && !isNeutralized
        }
    }
    private var isEmpty: Bool { tab?.url.isEmpty ?? true }
    /// An app-UI surface covers the canvas — the chip reads as inert, not the
    /// active tab's address (ADR-0022 follow-up, P1-21).
    private var isNeutralized: Bool { state.isSurfaceOpen }
    private var neutralIcon: String {
        if state.isHistoryOpen { return "clock" }
        if state.isSettingsOpen { return "gearshape" }
        return "key"
    }
    private var neutralTitle: String {
        if state.isHistoryOpen { return "History" }
        if state.isSettingsOpen { return "Settings" }
        return "Passwords"
    }
    private var text: String {
        guard let tab, !tab.url.isEmpty else { return "Search or enter URL" }
        return displayDomain(tab.url)
    }

    var body: some View {
        Group {
            switch placement {
            case .sidebar:
                sidebarBody
            case .viewport:
                viewportBody
            }
        }
        .onChange(of: state.isSurfaceOpen) { _, open in
            if open {
                showShield = false
                showVault = false
                hovering = false
                showViewportSuggestions = false
            }
        }
        .onChange(of: model.isEditing) { _, editing in
            if !editing { showViewportSuggestions = false }
        }
        .onChange(of: suggestions.suggestions.isEmpty) { _, empty in
            if placement == .viewport {
                showViewportSuggestions = model.isEditing && !empty
            }
        }
    }

    @ViewBuilder private var sidebarBody: some View {
        if hostsEditing {
            editor
        } else if isNeutralized {
            neutralChip
        } else {
            chip
        }
    }

    @ViewBuilder private var viewportBody: some View {
        // Transparent chrome — only the pill draws; the page viewport sits below
        // and can scroll up behind this strip (see `CEFContainerView`).
        HStack(spacing: 0) {
            Spacer(minLength: 0)
            viewportChrome
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 24)
        .padding(.top, 10)
        .padding(.bottom, 14)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(Color.clear)
        .zIndex(model.isEditing ? 1 : 0)
    }

    /// The in-page address pill plus its flanking vault and shield controls.
    @ViewBuilder private var viewportChrome: some View {
        HStack(spacing: 8) {
            if viewportVaultButtonVisible {
                VaultKeyButton(
                    host: tab?.host ?? "", isPresented: $showVault, placement: .viewport)
            }
            Group {
                if hostsEditing {
                    viewportEditor
                } else if isNeutralized {
                    viewportNeutralChip
                } else {
                    viewportChip
                }
            }
            if viewportShieldVisible { viewportShield }
        }
        // Hover drives the copy-link affordance inside the pill only.
        .onHover { hovering = $0 }
    }

    // ─── Neutral chip (surface open) ─────────────────────────────────────────

    private var neutralChip: some View {
        HStack(spacing: 6) {
            Image(systemName: neutralIcon)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.tertiary)
            Text(neutralTitle)
                .lineLimit(1)
                .foregroundStyle(.tertiary)
            Spacer(minLength: 0)
        }
        .font(.system(size: 13))
        .padding(.horizontal, 10)
        .frame(height: fieldHeight)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(.quaternary)
                .opacity(0.35)
        )
        .allowsHitTesting(false)
    }

    // ─── Display chip ──────────────────────────────────────────────────────────

    private var chip: some View {
        // The whole chip taps to edit (⌘L); the hover-revealed copy button is a
        // nested action, so — like TabRow's ✕ — the row uses `onTapGesture`
        // rather than a Button to keep the inner button's tap unambiguous.
        HStack(spacing: 6) {
            // Only the empty chip leads with an icon — the magnifying glass is a
            // search affordance ("type here"). A loaded URL shows the domain only
            // until focused (⌘L reveals the full address); no leading globe — the
            // tab carries the favicon, and site status lives in the shield.
            if isEmpty {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.secondary)
            }
            Text(text)
                .lineLimit(1)
                .truncationMode(.tail)
                .foregroundStyle(isEmpty ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
            Spacer(minLength: 0)
            if hovering, let url = tab?.url, !url.isEmpty {
                Button(action: { copyLink(url) }) {
                    Image(systemName: copied ? "checkmark" : "link")
                        .font(.system(size: 11, weight: .medium))
                        .contentTransition(.symbolEffect(.replace))
                        .frame(width: 18, height: 18)
                }
                .buttonStyle(IconButtonStyle(cornerRadius: 5, emphasizesLabel: false))
                .foregroundStyle(copied ? AnyShapeStyle(.green) : AnyShapeStyle(.secondary))
                .help("Copy Link")
            }
            if sidebarVaultButtonVisible {
                VaultKeyButton(host: tab?.host ?? "", isPresented: $showVault)
            }
            if sidebarShieldVisible { shield }
        }
        .font(.system(size: 13))
        .padding(.horizontal, 10)
        .frame(height: fieldHeight)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(.quaternary)
                .opacity(hovering ? 0.7 : 0.4)
                .hoverFade(hovering)
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

    // ─── Adblock shield (ADR-0020, style §16) ────────────────────────────────

    /// A real web page (not the empty new-tab chip / internal surfaces).
    private var isWebPage: Bool { (tab?.url.hasPrefix("http")) ?? false }

    /// Sidebar chip: surface the vault key when it has something to say, else on
    /// hover (P2-34). Keep it up while its popover is open.
    private var sidebarVaultButtonVisible: Bool {
        guard isWebPage, state.vaultConfigured else { return false }
        return !state.vaultMatches.isEmpty || hovering || showVault
    }

    /// In-page chrome: always show the vault control on real pages once set up.
    private var viewportVaultButtonVisible: Bool {
        isWebPage && state.vaultConfigured
    }

    /// Sidebar chip: surface the shield when it has something to say, else on
    /// hover. Keep it up while its popover is open.
    private var sidebarShieldVisible: Bool {
        isWebPage
            && (state.adblockBlockedCount > 0 || state.adblockSiteOff || hovering || showShield)
    }

    /// In-page chrome: always show the shield on real pages.
    private var viewportShieldVisible: Bool { isWebPage }

    private var shieldIcon: String {
        (!state.adblockEnabled || state.adblockSiteOff) ? "shield.slash" : "checkmark.shield.fill"
    }

    private var shieldTint: AnyShapeStyle {
        if !state.adblockEnabled { return AnyShapeStyle(.tertiary) }
        if state.adblockSiteOff { return AnyShapeStyle(.secondary) }
        if state.adblockBlockedCount > 0 { return AnyShapeStyle(Color.accentColor) }
        return AnyShapeStyle(.secondary)
    }

    private var shield: some View {
        Button {
            showShield.toggle()
        } label: {
            Image(systemName: shieldIcon)
                .font(.system(size: 12, weight: .semibold))
                .contentTransition(.symbolEffect(.replace))
                .foregroundStyle(shieldTint)
                .frame(width: 18, height: 18)
        }
        .buttonStyle(IconButtonStyle(cornerRadius: 5, emphasizesLabel: false))
        .help(shieldHelp)
        .popover(isPresented: $showShield, arrowEdge: .bottom) { shieldPopover }
    }

    /// The in-page shield control — its own Liquid Glass capsule beside the
    /// address pill. `onTapGesture` matches the pill's hit-test path under
    /// `PassthroughHostingView` (a `Button` does not register there).
    private var viewportShield: some View {
        Image(systemName: shieldIcon)
            .font(.system(size: 13, weight: .semibold))
            .contentTransition(.symbolEffect(.replace))
            .foregroundStyle(shieldTint)
            .frame(width: fieldHeight, height: fieldHeight)
            .background { ViewportGlassCapsule() }
            .contentShape(Capsule())
            .onTapGesture { showShield.toggle() }
            .help(shieldHelp)
            .popover(isPresented: $showShield, arrowEdge: .bottom) { shieldPopover }
    }

    private var shieldHelp: String {
        state.adblockSiteOff ? "Ad blocking off for this site" : "Ad & tracker blocking"
    }

    private var shieldCountText: String {
        if !state.adblockEnabled { return "Ad and tracker blocking is off." }
        if state.adblockSiteOff { return "Blocking is turned off for this site." }
        switch state.adblockBlockedCount {
        case 0: return "No ads or trackers blocked yet."
        case 1: return "1 request blocked on this page."
        default: return "\(state.adblockBlockedCount) requests blocked on this page."
        }
    }

    /// Cookie-consent is reject-first with no per-site override (ADR-0026), so
    /// this is a transparency line only — shown when there's something to
    /// report, not paired with a toggle.
    private var consentCountText: String? {
        guard state.consentEnabled, state.consentRejectedCount > 0 else { return nil }
        return state.consentRejectedCount == 1
            ? "1 cookie banner auto-rejected."
            : "\(state.consentRejectedCount) cookie banners auto-rejected."
    }

    private var shieldPopover: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(tab?.host.isEmpty == false ? tab!.host : "This site")
                .font(.headline)
                .lineLimit(1)
                .truncationMode(.middle)
            Label {
                Text(shieldCountText)
            } icon: {
                Image(systemName: shieldIcon).foregroundStyle(shieldTint)
            }
            .font(.subheadline)
            .foregroundStyle(.secondary)
            if let consentCountText {
                Label {
                    Text(consentCountText)
                } icon: {
                    Image(systemName: "hand.raised.fill").foregroundStyle(.secondary)
                }
                .font(.subheadline)
                .foregroundStyle(.secondary)
            }
            Divider()
            Toggle(
                "Block ads and trackers on this site",
                isOn: Binding(
                    get: { !state.adblockSiteOff },
                    set: { _ in state.toggleAdblockForSite() }
                )
            )
            .toggleStyle(.switch)
            .disabled(!state.adblockEnabled)
            if !state.adblockEnabled {
                Text("Ad blocking is off. Turn it on from the Shield menu.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(14)
        .frame(width: 268)
    }

    // ─── Inline editor ─────────────────────────────────────────────────────────

    private var editor: some View {
        // No leading icon — same as the loaded-URL chip. The magnifying glass is
        // only for the empty new-tab chip; inline edit-in-place (⌘L) is address
        // entry, not search.
        AutocompleteField(
            controller: suggestions,
            initialQuery: model.initialQuery,
            font: .system(size: 13),
            onFocusChange: { focused in
                // Clicking into the page (or elsewhere) resigns the field —
                // dismiss, matching the command bar losing key. Suggestion
                // taps don't take first responder, so they never trip this.
                if !focused { model.endEditing() }
            },
            selectAllOnFocus: true
        )
        .id(model.editingSession)
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
        .onKeyPress(.upArrow) {
            suggestions.moveSelection(by: -1)
            return .handled
        }
        .onKeyPress(.downArrow) {
            suggestions.moveSelection(by: 1)
            return .handled
        }
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
                autocompleteDropdown
            }
        }
    }

    private var autocompleteDropdown: some View {
        AutocompleteList(controller: suggestions, onChoose: { model.choose($0) })
            .padding(6)
            .background(
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .fill(Color(nsColor: .windowBackgroundColor))
                    .shadow(color: .black.opacity(0.18), radius: 12, y: 4)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1)
            )
            .fixedSize(horizontal: false, vertical: true)
            .offset(y: fieldHeight + 6)
    }

    // ─── Viewport chip / editor (Safari-style centered capsule) ──────────────

    private var viewportChip: some View {
        HStack(spacing: 8) {
            if isEmpty {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(.secondary)
            }
            Text(text)
                .lineLimit(1)
                .truncationMode(.middle)
                .foregroundStyle(isEmpty ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
            Spacer(minLength: 0)
            if hovering, let url = tab?.url, !url.isEmpty {
                Button(action: { copyLink(url) }) {
                    Image(systemName: copied ? "checkmark" : "link")
                        .font(.system(size: 12, weight: .medium))
                        .contentTransition(.symbolEffect(.replace))
                        .frame(width: 20, height: 20)
                }
                .buttonStyle(IconButtonStyle(cornerRadius: 6, emphasizesLabel: false))
                .foregroundStyle(copied ? AnyShapeStyle(.green) : AnyShapeStyle(.secondary))
                .help("Copy Link")
            }
        }
        .font(.system(size: 14))
        .padding(.horizontal, 18)
        .frame(maxWidth: ViewportURLBarMetrics.pillMaxWidth)
        .frame(height: fieldHeight)
        .background { ViewportGlassCapsule() }
        .clipShape(Capsule())
        .contentShape(Capsule())
        .onTapGesture(perform: activate)
        .help(isEmpty ? "New Tab (⌘T)" : "Edit URL (⌘L)")
    }

    private var viewportNeutralChip: some View {
        HStack(spacing: 8) {
            Image(systemName: neutralIcon)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(.tertiary)
            Text(neutralTitle)
                .lineLimit(1)
                .foregroundStyle(.tertiary)
            Spacer(minLength: 0)
        }
        .font(.system(size: 14))
        .padding(.horizontal, 18)
        .frame(maxWidth: ViewportURLBarMetrics.pillMaxWidth)
        .frame(height: fieldHeight)
        .background { ViewportGlassCapsule() }
        .clipShape(Capsule())
        .allowsHitTesting(false)
    }

    private var viewportEditor: some View {
        AutocompleteField(
            controller: suggestions,
            initialQuery: model.initialQuery,
            font: .system(size: 14),
            onFocusChange: { focused in
                if !focused { model.endEditing() }
            },
            selectAllOnFocus: true
        )
        .id(model.editingSession)
        .padding(.horizontal, 18)
        .frame(maxWidth: ViewportURLBarMetrics.pillMaxWidth)
        .frame(height: fieldHeight)
        .background(
            Capsule()
                .fill(Color(nsColor: .textBackgroundColor))
        )
        .overlay(
            Capsule()
                .strokeBorder(Color.accentColor, lineWidth: 2)
        )
        .onKeyPress(.upArrow) {
            suggestions.moveSelection(by: -1)
            return .handled
        }
        .onKeyPress(.downArrow) {
            suggestions.moveSelection(by: 1)
            return .handled
        }
        .onExitCommand { model.endEditing() }
        .onSubmit {
            if let selected = suggestions.selected() {
                model.choose(selected)
            } else {
                model.commit()
            }
        }
        .popover(isPresented: $showViewportSuggestions, arrowEdge: .bottom) {
            AutocompleteList(controller: suggestions, onChoose: { model.choose($0) })
                .padding(6)
                .frame(width: 400)
        }
    }

}
