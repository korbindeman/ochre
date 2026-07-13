import AppKit
import SwiftUI

/// Arc-style pinned tabs: spacious favicon icons below the URL bar, always
/// resident in memory (ADR-0032). Icons are permanent (ADR-0038) — clicking
/// focuses, ⌘W just deselects; only Unpin/Delete remove one.
struct PinnedTabStrip: View {
    @ObservedObject private var state = BrowserState.shared
    @ObservedObject var drag: TabDragModel
    @Namespace private var selection

    private let rowHeight: CGFloat = 40
    private let spacing: CGFloat = 6

    private var isDraggingFavorite: Bool {
        guard let id = drag.draggingId else { return false }
        return state.tabs.first { $0.id == id }?.favorite ?? false
    }

    var body: some View {
        let tabs = state.favoriteTabs
        Group {
            if tabs.isEmpty {
                if drag.draggingId != nil, !isDraggingFavorite, state.canPinMoreFavorites {
                    emptyPinDropZone
                }
            } else {
                let layout = PinnedGridLayout.forTabCount(tabs.count)
                VStack(alignment: .leading, spacing: spacing) {
                    ForEach(0..<layout.rows, id: \.self) { row in
                        pinnedRow(row, layout: layout, tabs: tabs)
                    }
                }
                .padding(.vertical, 2)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        // The band the resolver treats as favorites territory (horizontal slots).
        .reportSidebarDropFrame(.favoriteStrip)
    }

    @ViewBuilder
    private func pinnedRow(_ row: Int, layout: PinnedGridLayout, tabs: [TabItem]) -> some View {
        let items = Self.itemsInRow(row, layout: layout, tabs: tabs)

        HStack(spacing: spacing) {
            ForEach(items) { tab in
                pinnedButton(tab)
                    .slotFrame(height: rowHeight)
                    .reportSidebarDropFrame(.favorite(tab.id))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private func pinnedButton(_ tab: TabItem) -> some View {
        let isActive = tab.id == state.activeTabId && !state.isSurfaceOpen
        PinnedTabButton(
            tab: tab,
            isActive: isActive,
            selection: selection,
            onSelect: { state.handleTabClick(tab.id) }
        )
        .contextMenu { PinnedTabMenu(tab: tab, isActive: isActive) }
        .sidebarTabDrag(tab: tab, drag: drag)
        .transition(SidebarSelection.rowTransition)
    }

    /// Pin-zone affordance when the strip is empty but a pinnable tab is
    /// mid-drag — lit while the drag is aimed at it. Purely visual; the reported
    /// strip frame is what the resolver targets.
    @ViewBuilder
    private var emptyPinDropZone: some View {
        FavoriteDropPlaceholder()
            .opacity(drag.slot == .endOf(.favorite) ? 1 : 0.35)
            .frame(height: rowHeight)
            .frame(maxWidth: .infinity)
    }

    /// One row's tabs in store order.
    private static func itemsInRow(
        _ row: Int,
        layout: PinnedGridLayout,
        tabs: [TabItem]
    ) -> [TabItem] {
        let start = layout.startIndex(forRow: row)
        guard start < tabs.count else { return [] }
        let end = min(start + layout.columns(inRow: row), tabs.count)
        return Array(tabs[start..<end])
    }

}

extension View {
    /// Equal share of the row — 1 tab fills the strip, 2 ⇒ 50%, 3 ⇒ 33%, etc.
    fileprivate func slotFrame(height: CGFloat) -> some View {
        frame(maxWidth: .infinity).frame(height: height)
    }
}

/// Per-row layout for the pinned strip — driven by tab count, not sidebar width.
private struct PinnedGridLayout {
    /// Slot count per row, e.g. five tabs ⇒ `[2, 3]`.
    let slotsPerRow: [Int]

    var rows: Int { slotsPerRow.count }

    /// Matches [`BrowserState.maxFavoriteTabs`] / core `MAX_VISIBLE_FAVORITES`.
    static let maxTabs = 6

    func columns(inRow row: Int) -> Int { slotsPerRow[row] }

    func startIndex(forRow row: Int) -> Int {
        slotsPerRow.prefix(row).reduce(0, +)
    }

    /// 1 ⇒ 100%, 2 ⇒ 50%, 3 ⇒ 33%, 4 ⇒ 2×2, 5 ⇒ 2+3, 6 ⇒ 3×2.
    static func forTabCount(_ tabCount: Int) -> PinnedGridLayout {
        guard tabCount > 0 else { return PinnedGridLayout(slotsPerRow: []) }
        let slots: [Int] =
            switch tabCount {
            case 1: [1]
            case 2: [2]
            case 3: [3]
            case 4: [2, 2]
            case 5: [2, 3]
            case 6: [3, 3]
            default: [3, 3]
            }
        return PinnedGridLayout(slotsPerRow: slots)
    }
}

/// One pinned-tab icon — favicon-centered, active pill. The icon is permanent
/// (ADR-0038): click to focus, ⌘W to deselect; unpin/delete via the context menu.
private struct PinnedTabButton: View {
    let tab: TabItem
    let isActive: Bool
    let selection: Namespace.ID
    let onSelect: () -> Void

    @Environment(\.colorScheme) private var scheme
    @State private var hovering = false
    /// Dominant favicon hue, `nil` for monochrome icons (then the neutral floor
    /// carries presence on its own). Resolved from the same cached favicon image.
    @State private var tint: Color?

    var body: some View {
        TabFavicon(tab: tab, size: 18)
            .frame(width: 20, height: 20)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(chip)
            .contentShape(.rect)
            .onTapGesture(perform: onSelect)
            .onHover { hovering = $0 }
            .help(tab.displayTitle)
            .task(id: "\(tab.host)#\(tab.faviconVersion)") {
                let ns = tab.host.isEmpty ? nil : await FaviconLoader.shared.dominantColor(for: tab.host)
                tint = ns.map(Color.init(nsColor:))
            }
    }

    /// Rest → hover → active as a presence ladder: a persistent neutral floor gives
    /// every tile a body even at rest (fixing the "invisible favicon" problem), a
    /// favicon-tinted wash layers identity where the icon has a real hue, and the
    /// active tile gains a matched ring that slides between tiles.
    @ViewBuilder private var chip: some View {
        let dark = scheme == .dark
        RoundedRectangle(cornerRadius: 10)
            .fill(.primary.opacity(floorOpacity))
            .hoverFade(hovering)
        if let tint {
            RoundedRectangle(cornerRadius: 10)
                .fill(tint.opacity(washOpacity(dark: dark)))
                .hoverFade(hovering)
        }
        if isActive {
            RoundedRectangle(cornerRadius: 10)
                .strokeBorder((tint ?? .primary).opacity(tint == nil ? 0.25 : 0.55), lineWidth: 1.5)
                .matchedGeometryEffect(id: SidebarSelection.pill, in: selection)
        }
    }

    /// Neutral floor: always present; a touch stronger when active, and stronger
    /// still for monochrome tiles (no wash to lean on).
    private var floorOpacity: Double {
        if isActive { return tint == nil ? 0.16 : 0.10 }
        return hovering ? 0.10 : 0.06
    }

    /// Favicon wash: dark sidebars need more of it to read at the same presence.
    private func washOpacity(dark: Bool) -> Double {
        if isActive { return dark ? 0.42 : 0.28 }
        let base = dark ? 0.30 : 0.16
        return hovering ? base + 0.06 : base
    }
}

/// Context menu for a pinned-tab icon.
private struct PinnedTabMenu: View {
    let tab: TabItem
    let isActive: Bool
    private var app: AppDelegate? { NSApp.delegate as? AppDelegate }

    var body: some View {
        Button("Unpin Tab") { app?.setTabFavorite(tab.id, false) }
        Divider()
        if isActive {
            Button("Close Pinned Tab") { app?.closeTab(tab.id) }
        }
        Button("Delete Pinned Tab") { app?.deleteTab(tab.id) }
    }
}

