import AppKit
import SwiftUI

/// Which sidebar instance this is — the pinned column (always laid out) or the
/// hover-revealed floating copy. Only the visible one hosts the live URL editor
/// (see `URLBar.isVisible`).
enum SidebarVariant { case pinned, floating }

/// Sidebar chrome content (hosted in an `NSHostingView` island — see `RootView`
/// for placement). The top inset clears the traffic lights, which overlay this
/// region (the window uses `.fullSizeContentView` with a hidden title).
///
/// The tab list is bound to the core-owned `BrowserState`: selecting a row
/// activates that tab, the ✕ closes it, and titles / favicons / spinners update
/// live from the event stream.
struct Sidebar: View {
    var variant: SidebarVariant = .pinned
    var isPrivate: Bool = false
    @ObservedObject private var state = BrowserState.shared

    /// The on-screen instance: the pinned column when pinned, the floating copy
    /// when unpinned-and-revealed. Drives which sidebar owns the inline URL field.
    private var isVisible: Bool {
        switch variant {
        case .pinned: return state.isSidebarPinned
        case .floating: return state.isFloatingSidebarShown
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            // Controls share the traffic lights' row (Arc): the custom lights
            // lead (see TrafficLights), the sidebar toggle sits just after them,
            // and back/forward/reload group at the right edge.
            HStack(spacing: 2) {
                TrafficLights()
                NavButton(symbol: "sidebar.left", help: "Toggle Sidebar (⌘S)") {
                    (NSApp.delegate as? AppDelegate)?.toggleSidebar()
                }
                Spacer()
                NavButton(
                    symbol: "chevron.left", help: "Back",
                    disabled: !(state.activeTab?.canGoBack ?? false)
                ) { browser_go_back() }
                NavButton(
                    symbol: "chevron.right", help: "Forward",
                    disabled: !(state.activeTab?.canGoForward ?? false)
                ) { browser_go_forward() }
                ReloadButton()
            }
            .padding(.top, 13)

            // Lifted above the tab list so its inline autocomplete drops down as a
            // floating card without reflowing the rows beneath it.
            URLBar(isVisible: isVisible)
                .zIndex(1)

            if !isPrivate {
                PinnedTabStrip()
            }

            // The active space's tree: saved section (folders + loose saved)
            // above New Tab + open tabs. Scrolls independently.
            // Fill the scroll viewport so empty list space is tappable — clicking
            // it dismisses the inline URL editor (the field only resigns on its
            // own when focus moves elsewhere, which bare sidebar chrome doesn't).
            GeometryReader { proxy in
                ScrollView {
                    SpaceContents()
                        .frame(
                            maxWidth: .infinity, minHeight: proxy.size.height,
                            alignment: .topLeading
                        )
                        .contentShape(.rect)
                        .onTapGesture {
                            URLBarModel.shared.endEditing()
                            BrowserState.shared.clearTabSelection()
                        }
                }
                .scrollIndicators(.never)
            }
            .frame(maxHeight: .infinity)

            if isPrivate {
                HStack(spacing: 6) {
                    Image(systemName: "hand.raised.fill")
                        .font(.system(size: 11))
                    Text("Private")
                        .font(.system(size: 12, weight: .semibold))
                }
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, 4)
            } else {
                SpaceSwitcher()
            }
        }
        .padding(.horizontal, 12)
        .padding(.bottom, 10)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background {
            Color.clear
                .contentShape(.rect)
                .onTapGesture {
                    URLBarModel.shared.endEditing()
                    BrowserState.shared.clearTabSelection()
                }
        }
    }
}

struct NavButton: View {
    let symbol: String
    let help: String
    /// Dims the glyph and blocks the action + hover/press highlight — used for the
    /// back/forward arrows when there's no history to move through.
    var disabled: Bool = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: .medium))
                // Reload⇄stop (and any future symbol swap) cross-fades via Magic
                // Replace; a no-op for the static back/forward/sidebar glyphs.
                .contentTransition(.symbolEffect(.replace))
                .frame(width: 30, height: 26)
        }
        // Fill on hover, deeper on press; glyph secondary → primary, quaternary when
        // disabled (dead control) with the fill suppressed — all via IconButtonStyle.
        .buttonStyle(IconButtonStyle(cornerRadius: 6))
        .disabled(disabled)
        .help(help)
    }
}

/// Reload / stop toggle: while the active tab is loading it shows a ✕ that
/// cancels the load, and a refresh glyph otherwise — the standard browser
/// combined button. Observes state directly so it flips the moment loading does.
struct ReloadButton: View {
    @ObservedObject private var state = BrowserState.shared

    var body: some View {
        let loading = state.activeTab?.isLoading ?? false
        NavButton(
            symbol: loading ? "xmark" : "arrow.clockwise",
            help: loading ? "Stop" : "Reload"
        ) {
            if loading { browser_stop_load() } else { browser_reload() }
        }
    }
}
