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
    /// One drag session for the pinned strip and the tab list beneath it.
    @StateObject private var tabDrag = TabDragModel()

    /// The on-screen instance: the pinned column when pinned, the floating copy
    /// when unpinned-and-revealed. Drives which sidebar owns the inline URL field.
    private var isVisible: Bool {
        switch variant {
        case .pinned: return state.isSidebarPinned
        case .floating: return state.isFloatingSidebarShown
        }
    }

    /// One space's scrolling tree, sized to a filmstrip cell. `interactive` is true
    /// only for the active space's panel: the inert panels render their rows for the
    /// slide but wire no drag, drop-frame reporting, or taps, so the sidebar keeps
    /// exactly one live list (see `SpaceContents`). Empty scroll space taps dismiss
    /// the inline URL editor and clear the selection (only the active panel resigns
    /// focus; bare chrome otherwise never would).
    @ViewBuilder
    private func spacePanel(spaceId: Int32, interactive: Bool, width: CGFloat, height: CGFloat)
        -> some View
    {
        ScrollView {
            SpaceContents(spaceId: spaceId, interactive: interactive, drag: tabDrag)
                .frame(maxWidth: .infinity, minHeight: height, alignment: .topLeading)
                .contentShape(.rect)
                .onTapGesture {
                    guard interactive else { return }
                    URLBarModel.shared.endEditing()
                    BrowserState.shared.clearTabSelection()
                }
        }
        .scrollIndicators(.never)
        .frame(width: width)
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
            .windowDragHandleBackground()

            // Lifted above the tab list so its inline autocomplete drops down as a
            // floating card without reflowing the rows beneath it. Hidden when the
            // address bar floats over the page viewport instead (Safari-style).
            if !state.isViewportURLBarEnabled {
                URLBar(placement: .sidebar, isVisible: isVisible)
                    .windowDragHandleBackground()
                    .zIndex(1)
            }

            if !isPrivate {
                PinnedTabStrip(drag: tabDrag)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }

            // The spaces' trees as a horizontal filmstrip: every space is laid out
            // side by side, one panel each, and the strip is offset to the active
            // one. A switch animates that offset, so the outgoing and incoming lists
            // always travel together in the switcher's left→right direction — a real
            // slide, immune to the transition-capture quirks that made an `.id`-swap
            // slide flip or cross-fade on reversals (ADR-0043). Only the active panel
            // is interactive; the rest are inert display so they can't pollute the
            // Y-based drop resolver. Private windows have no spaces — one static panel.
            GeometryReader { proxy in
                let w = proxy.size.width
                if isPrivate {
                    spacePanel(spaceId: 0, interactive: true, width: w, height: proxy.size.height)
                } else {
                    let activeIndex = state.spaces.firstIndex { $0.id == state.activeSpaceId } ?? 0
                    HStack(spacing: 0) {
                        ForEach(state.spaces) { space in
                            spacePanel(
                                spaceId: space.id,
                                interactive: space.id == state.activeSpaceId,
                                width: w,
                                height: proxy.size.height
                            )
                        }
                    }
                    .frame(width: w, alignment: .leading)
                    // The strip offset rides `applyStructure`'s animation transaction
                    // (the same structural spring that springs every sidebar change):
                    // it snaps on first population and under Reduce Motion because that
                    // path applies the change bare, and slides on a real switch. No
                    // separate `.animation(value:)` — that also fires on the launch
                    // hydration, sliding the initial space into place (ADR-0043).
                    .offset(x: -CGFloat(activeIndex) * w)
                }
            }
            .frame(maxHeight: .infinity)
            .clipped()
            // The scroll viewport — the drag model autoscrolls at its edges.
            .reportSidebarDropFrame(.viewport)

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
        .background { SidebarPageTintBackground(isPrivate: isPrivate) }
        .sidebarPageTintColorScheme(isPrivate: isPrivate)
        // One coordinate space for the whole tree (ADR-0035): strip icons, rows,
        // the drag gesture, and the overlay's indicator/ghost all speak in it.
        // Released outside the root frame cancels the drag.
        .reportSidebarDropFrame(.root)
        .coordinateSpace(.named(SidebarDropSpace.name))
        .overlay { SidebarDragOverlay(drag: tabDrag, geometry: tabDrag.geometry) }
        .onPreferenceChange(SidebarDropFramesKey.self) { frames in
            MainActor.assumeIsolated { tabDrag.framesChanged(frames) }
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
