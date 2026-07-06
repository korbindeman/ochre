import AppKit
import SwiftUI

/// Sidebar chrome content (hosted in an `NSHostingView` island — see `RootView`
/// for placement). The top inset clears the traffic lights, which overlay this
/// region (the window uses `.fullSizeContentView` with a hidden title).
///
/// The tab list is bound to the core-owned `BrowserState`: selecting a row
/// activates that tab, the ✕ closes it, and titles / favicons / spinners update
/// live from the event stream.
/// Which sidebar instance this is — the pinned column (always laid out) or the
/// hover-revealed floating copy. Only the visible one hosts the live URL editor
/// (see `URLBar.isVisible`).
enum SidebarVariant { case pinned, floating }

struct Sidebar: View {
    var variant: SidebarVariant = .pinned
    @ObservedObject private var state = BrowserState.shared

    /// The on-screen instance: the pinned column when pinned, the floating copy
    /// when unpinned-and-revealed. Drives which sidebar owns the inline URL field.
    private var isVisible: Bool {
        switch variant {
        case .pinned: return state.isSidebarPinned
        case .floating: return !state.isSidebarPinned && state.isSidebarRevealed
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

            // The active space's tree: pinned section (folders + loose pinned)
            // above New Tab + the today (ephemeral) tabs. Scrolls independently.
            ScrollView {
                SpaceContents()
            }
            .scrollIndicators(.never)

            // Space switcher lives at the bottom, Arc-style.
            SpaceSwitcher()
        }
        .padding(.horizontal, 12)
        .padding(.bottom, 10)
    }
}

/// A single vertical tab: favicon (or spinner while loading), title, and a
/// hover-revealed close button. The whole row selects; only the ✕ closes.
/// (Non-private so the spaces/folders tree in `Spaces.swift` reuses it.)
struct TabRow: View {
    let tab: TabItem
    let isActive: Bool
    let onSelect: () -> Void
    let onClose: () -> Void

    @State private var hovering = false

    var body: some View {
        HStack(spacing: 8) {
            TabFavicon(tab: tab)
                .frame(width: 16, height: 16)
            Text(tab.displayTitle)
                .lineLimit(1)
            Spacer(minLength: 0)
            if hovering {
                Button(action: onClose) {
                    Image(systemName: "xmark")
                        .font(.system(size: 10, weight: .bold))
                        .frame(width: 18, height: 18)
                        .contentShape(.rect(cornerRadius: 5))
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .help("Close Tab")
            }
        }
        .font(.system(size: 13, weight: .medium))
        .padding(.horizontal, 10)
        .padding(.vertical, 9)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(.quaternary)
                .opacity(isActive ? 1 : (hovering ? 0.45 : 0))
        )
        .contentShape(.rect(cornerRadius: 8))
        .onTapGesture(perform: onSelect)
        .onHover { hovering = $0 }
    }
}

/// A tab's favicon, resolved asynchronously through `FaviconLoader` and keyed by
/// host + favicon version so it refreshes on navigation and on capture. Shows a
/// spinner while the page loads and a globe glyph when no icon is available.
struct TabFavicon: View {
    let tab: TabItem
    @State private var image: NSImage?

    var body: some View {
        Group {
            if tab.isLoading {
                ProgressView()
                    .controlSize(.small)
                    .scaleEffect(0.6)
            } else if let image {
                Image(nsImage: image)
                    .resizable()
                    .frame(width: 16, height: 16)
            } else {
                Image(systemName: "globe")
                    .foregroundStyle(.secondary)
            }
        }
        .task(id: "\(tab.host)#\(tab.faviconVersion)") {
            image = tab.host.isEmpty ? nil : await FaviconLoader.shared.icon(for: tab.host)
        }
    }
}

/// Shown in the content region when there are no tabs — the launch state and
/// the state after the last tab closes. Fully clean by design: a single, quiet
/// New-Tab hint on an otherwise empty canvas (a new-tab *page* with suggestions
/// is Phase 2).
struct EmptyStateView: View {
    @State private var hovering = false

    var body: some View {
        Button {
            (NSApp.delegate as? AppDelegate)?.toggleCommandBar()
        } label: {
            HStack(spacing: 8) {
                Text("New Tab")
                Text("⌘T")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(.tertiary)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(RoundedRectangle(cornerRadius: 5).fill(.quaternary).opacity(0.7))
            }
            .font(.system(size: 14, weight: .medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
            .background(RoundedRectangle(cornerRadius: 11).fill(.quaternary).opacity(hovering ? 0.5 : 0))
            .contentShape(.rect(cornerRadius: 11))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .windowBackgroundColor))
    }
}

/// The unpinned sidebar's content, shown floating over the web view. Solid
/// background on purpose: translucent materials sample their backdrop, and
/// CEF's content is a GPU-hosted remote layer that in-window blur cannot
/// sample reliably.
struct FloatingSidebar: View {
    var body: some View {
        Sidebar(variant: .floating)
            .frame(maxHeight: .infinity)
            .background {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(Color(nsColor: .windowBackgroundColor))
                    .shadow(color: .black.opacity(0.25), radius: 16, x: 3, y: 0)
            }
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1)
            }
    }
}

private struct NavButton: View {
    let symbol: String
    let help: String
    /// Dims the glyph and blocks the action + hover highlight — used for the
    /// back/forward arrows when there's no history to move through.
    var disabled: Bool = false
    let action: () -> Void

    @State private var hovering = false

    /// Enabled glyph brightens on hover (secondary → primary); disabled stays
    /// quaternary so the dead control reads as unavailable, not merely quiet.
    private var tint: AnyShapeStyle {
        if disabled { return AnyShapeStyle(.quaternary) }
        return hovering ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary)
    }

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: .medium))
                .frame(width: 30, height: 26)
                .background(
                    RoundedRectangle(cornerRadius: 6)
                        .fill(.quaternary)
                        .opacity(hovering && !disabled ? 0.7 : 0)
                )
                .contentShape(.rect(cornerRadius: 6))
        }
        .buttonStyle(.plain)
        .foregroundStyle(tint)
        .disabled(disabled)
        .onHover { hovering = $0 }
        .help(help)
    }
}

/// Reload / stop toggle: while the active tab is loading it shows a ✕ that
/// cancels the load, and a refresh glyph otherwise — the standard browser
/// combined button. Observes state directly so it flips the moment loading does.
private struct ReloadButton: View {
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

/// Invisible hover sensor (plain AppKit). Tracking areas keep working while
/// `hitTest` returns nil, so clicks pass through to whatever is beneath.
final class TrackingView: NSView {
    var onChange: ((Bool) -> Void)?

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(
            rect: .zero,
            options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect],
            owner: self,
            userInfo: nil
        ))
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func mouseEntered(with event: NSEvent) { onChange?(true) }
    override func mouseExited(with event: NSEvent) { onChange?(false) }
}

/// The `NSView` handed to CEF for a single tab. CEF inserts its GPU-hosted web
/// content as a subview; keep it filling the container on resize. Creation is
/// driven explicitly by `RootView.openNewTab` (via `browser_tab_create`) — the
/// container never creates a browser on its own.
final class CEFContainerView: NSView {
    /// Remove CEF's child view so it deallocates and this tab's browser teardown
    /// runs (CEF finishes destroying a browser only once its host view releases).
    func detachCEFView() {
        for subview in subviews {
            subview.removeFromSuperview()
        }
    }

    // CEF adds its web content as a subview; keep it filling the container on
    // every resize so the page tracks the window.
    override func layout() {
        super.layout()
        for subview in subviews {
            subview.frame = bounds
        }
    }
}
