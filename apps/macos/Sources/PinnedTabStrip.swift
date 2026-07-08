import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Arc-style pinned tabs: spacious favicon icons below the URL bar, always
/// resident in memory (ADR-0032). A user close dismisses the icon only.
struct PinnedTabStrip: View {
    @ObservedObject private var state = BrowserState.shared
    @ObservedObject var drag: TabDragModel
    @Namespace private var selection

    private var app: AppDelegate? { NSApp.delegate as? AppDelegate }

    private let rowHeight: CGFloat = 40
    private let spacing: CGFloat = 6

    private var isDraggingFavorite: Bool {
        guard let id = drag.draggingId else { return false }
        return state.tabs.first { $0.id == id }?.favorite ?? false
    }

    var body: some View {
        let tabs = state.favoriteTabs
        if tabs.isEmpty {
            if drag.draggingId != nil, !isDraggingFavorite {
                emptyPinDropZone
            }
        } else {
            let layout = PinnedGridLayout.forCount(tabs.count)
            VStack(alignment: .leading, spacing: spacing) {
                ForEach(0..<layout.rows, id: \.self) { row in
                    pinnedRow(row, layout: layout, tabs: tabs)
                }
            }
            .padding(.vertical, 2)
            .frame(maxWidth: .infinity, alignment: .leading)
            .blocksWindowDragPassthrough()
        }
    }

    @ViewBuilder
    private func pinnedRow(_ row: Int, layout: PinnedGridLayout, tabs: [TabItem]) -> some View {
        let items = Self.itemsInRow(row, columns: layout.columns, tabs: tabs)
        let isLastRow = row == layout.rows - 1

        HStack(spacing: spacing) {
            ForEach(items, id: \.tab.id) { item in
                pinnedButton(item.tab, next: item.nextId)
                    .frame(maxWidth: .infinity)
                    .frame(height: rowHeight)
            }
            if isLastRow {
                stripDropTarget()
            }
            if isLastRow, !state.dismissedFavorites.isEmpty {
                dismissedMenu(state.dismissedFavorites)
            }
        }
    }

    @ViewBuilder
    private func pinnedButton(_ tab: TabItem, next: Int32?) -> some View {
        PinnedTabButton(
            tab: tab,
            isActive: tab.id == state.activeTabId && !state.isSurfaceOpen,
            selection: selection,
            onSelect: { state.handleTabClick(tab.id) }
        )
        .contextMenu { PinnedTabMenu(tab: tab) }
        .pinnedTabDrag(tab: tab, next: next, drag: drag)
        .transition(SidebarSelection.rowTransition)
    }

    /// Full-width target when the strip is empty but a sidebar tab is mid-drag.
    @ViewBuilder
    private var emptyPinDropZone: some View {
        Color.clear
            .frame(height: rowHeight)
            .frame(maxWidth: .infinity)
            .contentShape(.rect)
            .overlay {
                RoundedRectangle(cornerRadius: 10)
                    .strokeBorder(Color.accentColor, lineWidth: 2)
                    .opacity(drag.slot == .endOf(.favorite) ? 1 : 0.35)
            }
            .onDrop(of: [.text], delegate: FavoriteStripDropDelegate(drag: drag))
            .blocksWindowDragPassthrough()
    }

    /// Trailing drop target on the last row — pins a dragged tab at the end.
    @ViewBuilder
    private func stripDropTarget() -> some View {
        Color.clear
            .frame(width: 12, height: rowHeight)
            .contentShape(.rect)
            .overlay(alignment: .leading) {
                VerticalInsertionIndicator()
                    .opacity(drag.slot == .endOf(.favorite) ? 1 : 0)
            }
            .onDrop(
                of: [.text],
                delegate: FavoriteStripDropDelegate(drag: drag)
            )
    }

    @ViewBuilder
    private func dismissedMenu(_ tabs: [TabItem]) -> some View {
        Menu {
            ForEach(tabs) { tab in
                Button(tab.displayTitle) { app?.restoreTabFavorite(tab.id) }
            }
        } label: {
            Image(systemName: "ellipsis")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 28, height: rowHeight)
        }
        .menuStyle(.borderlessButton)
        .help("Closed pinned tabs")
    }

    /// One row's tabs in store order, each with its reorder successor.
    private static func itemsInRow(
        _ row: Int,
        columns: Int,
        tabs: [TabItem]
    ) -> [(tab: TabItem, nextId: Int32?)] {
        let start = row * columns
        guard start < tabs.count else { return [] }
        let end = min(start + columns, tabs.count)
        return (start..<end).map { index in
            let next = index + 1 < tabs.count ? tabs[index + 1].id : nil
            return (tabs[index], next)
        }
    }
}

/// How many columns and rows the pinned strip uses for a given tab count.
private struct PinnedGridLayout {
    let columns: Int
    let rows: Int

    static func forCount(_ count: Int) -> PinnedGridLayout {
        guard count > 0 else { return PinnedGridLayout(columns: 0, rows: 0) }
        if count <= 3 {
            return PinnedGridLayout(columns: count, rows: 1)
        }
        let columns = Int(ceil(sqrt(Double(count))))
        let rows = Int(ceil(Double(count) / Double(columns)))
        return PinnedGridLayout(columns: columns, rows: rows)
    }
}

/// One pinned-tab icon — favicon-centered, active pill. Dismiss via drag-out or
/// the context menu, not a hover close affordance.
private struct PinnedTabButton: View {
    let tab: TabItem
    let isActive: Bool
    let selection: Namespace.ID
    let onSelect: () -> Void

    @State private var hovering = false

    var body: some View {
        TabFavicon(tab: tab)
            .frame(width: 20, height: 20)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background {
                RoundedRectangle(cornerRadius: 10)
                    .fill(.quaternary)
                    .opacity(!isActive && hovering ? 0.45 : 0)
                    .hoverFade(hovering)
                if isActive {
                    RoundedRectangle(cornerRadius: 10)
                        .fill(.quaternary)
                        .matchedGeometryEffect(id: SidebarSelection.pill, in: selection)
                }
            }
            .contentShape(.rect(cornerRadius: 10))
            .onTapGesture(perform: onSelect)
            .onHover { hovering = $0 }
            .help(tab.displayTitle)
    }
}

/// Context menu for a pinned-tab icon.
private struct PinnedTabMenu: View {
    let tab: TabItem
    private var app: AppDelegate? { NSApp.delegate as? AppDelegate }

    var body: some View {
        Button("Unpin Tab") { app?.setTabFavorite(tab.id, false) }
        Divider()
        Button("Close Pinned Tab") { app?.closeTab(tab.id) }
    }
}

/// Accent line for horizontal reordering in the pinned strip.
struct VerticalInsertionIndicator: View {
    var body: some View {
        Capsule()
            .fill(Color.accentColor)
            .frame(width: 2.5)
            .padding(.vertical, 6)
    }
}

/// Horizontal drag/drop for one pinned-tab icon.
private struct PinnedTabDrag: ViewModifier {
    let tab: TabItem
    let next: Int32?
    @ObservedObject var drag: TabDragModel

    @State private var width: CGFloat = 40

    private var slotBefore: DropSlot { .before(tab.id) }
    private var slotAfter: DropSlot { next.map { .before($0) } ?? .endOf(.favorite) }

    func body(content: Content) -> some View {
        content
            .opacity(drag.draggingId == tab.id ? 0.35 : 1)
            .background(
                GeometryReader { geo in
                    Color.clear.preference(key: PinnedTabWidthKey.self, value: geo.size.width)
                }
            )
            .onPreferenceChange(PinnedTabWidthKey.self) { width = $0 }
            .overlay(alignment: .leading) {
                VerticalInsertionIndicator().opacity(drag.slot == slotBefore ? 1 : 0)
            }
            .overlay(alignment: .trailing) {
                VerticalInsertionIndicator()
                    .opacity(next == nil && drag.slot == .endOf(.favorite) ? 1 : 0)
            }
            .onDrag {
                let id = tab.id
                DispatchQueue.main.async { drag.begin(id) }
                return NSItemProvider(object: String(id) as NSString)
            }
            .onDrop(
                of: [.text],
                delegate: PinnedTabDropDelegate(
                    tabId: tab.id,
                    slotBefore: slotBefore,
                    slotAfter: slotAfter,
                    tabWidth: { width },
                    drag: drag
                )
            )
    }
}

private struct PinnedTabWidthKey: PreferenceKey {
    static let defaultValue: CGFloat = 40
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = nextValue() }
}

private struct PinnedTabDropDelegate: DropDelegate {
    let tabId: Int32
    let slotBefore: DropSlot
    let slotAfter: DropSlot
    let tabWidth: () -> CGFloat
    let drag: TabDragModel

    func validateDrop(info: DropInfo) -> Bool { drag.draggingId != nil }
    func dropEntered(info: DropInfo) { update(info) }
    func dropUpdated(info: DropInfo) -> DropProposal? {
        update(info)
        return DropProposal(operation: .move)
    }
    func dropExited(info: DropInfo) {
        if drag.slot == slotBefore || drag.slot == slotAfter { drag.slot = nil }
    }
    func performDrop(info: DropInfo) -> Bool {
        update(info)
        drag.commit()
        return true
    }

    private func update(_ info: DropInfo) {
        guard let dragging = drag.draggingId, tabId != dragging else {
            drag.slot = nil
            return
        }
        let slot = info.location.x < tabWidth() / 2 ? slotBefore : slotAfter
        drag.slot = slot == .before(dragging) ? nil : slot
    }
}

/// Trailing drop zone on the last row for pinning from other sections.
private struct FavoriteStripDropDelegate: DropDelegate {
    let drag: TabDragModel

    func validateDrop(info: DropInfo) -> Bool { drag.draggingId != nil }
    func dropEntered(info: DropInfo) { drag.slot = .endOf(.favorite) }
    func dropUpdated(info: DropInfo) -> DropProposal? {
        drag.slot = .endOf(.favorite)
        return DropProposal(operation: .move)
    }
    func dropExited(info: DropInfo) {
        if drag.slot == .endOf(.favorite) { drag.slot = nil }
    }
    func performDrop(info: DropInfo) -> Bool {
        drag.slot = .endOf(.favorite)
        drag.commit()
        return true
    }
}

extension View {
    func pinnedTabDrag(tab: TabItem, next: Int32?, drag: TabDragModel) -> some View {
        modifier(PinnedTabDrag(tab: tab, next: next, drag: drag))
    }
}
