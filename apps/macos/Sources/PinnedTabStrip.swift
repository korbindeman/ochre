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
    @State private var stripWidth: CGFloat = 0

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
            let layout = PinnedGridLayout.forWidth(
                stripWidth, tabCount: tabs.count, itemSize: rowHeight, spacing: spacing
            )
            VStack(alignment: .leading, spacing: spacing) {
                ForEach(0..<layout.rows, id: \.self) { row in
                    pinnedRow(row, layout: layout, tabs: tabs)
                }
            }
            .padding(.vertical, 2)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background {
                GeometryReader { geo in
                    Color.clear.preference(key: PinnedStripWidthKey.self, value: geo.size.width)
                }
            }
            .onPreferenceChange(PinnedStripWidthKey.self) { stripWidth = $0 }
        }
    }

    @ViewBuilder
    private func pinnedRow(_ row: Int, layout: PinnedGridLayout, tabs: [TabItem]) -> some View {
        let items = Self.itemsInRow(row, columns: layout.columns, tabs: tabs)
        let isLastRow = row == layout.rows - 1

        HStack(spacing: spacing) {
            ForEach(items, id: \.tab.id) { item in
                if drag.slot == .before(item.tab.id) {
                    FavoriteDropPlaceholder()
                        .frame(width: rowHeight, height: rowHeight)
                }
                pinnedButton(item.tab, next: item.nextId)
                    .frame(width: rowHeight, height: rowHeight)
            }
            if isLastRow, drag.slot == .endOf(.favorite) {
                FavoriteDropPlaceholder()
                    .frame(width: rowHeight, height: rowHeight)
            }
            if isLastRow, drag.draggingId != nil {
                stripDropTarget()
            }
            if isLastRow, !state.dismissedFavorites.isEmpty {
                dismissedMenu(state.dismissedFavorites)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
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
        Group {
            if drag.slot == .endOf(.favorite) {
                FavoriteDropPlaceholder()
            } else {
                FavoriteDropPlaceholder()
                    .opacity(0.35)
            }
        }
        .frame(height: rowHeight)
        .frame(maxWidth: .infinity)
        .contentShape(.rect)
        .onDrop(of: [.text], delegate: FavoriteStripDropDelegate(drag: drag))
    }

    /// Trailing drop target on the last row — pins a dragged tab at the end.
    @ViewBuilder
    private func stripDropTarget() -> some View {
        Color.clear
            .frame(width: 12, height: rowHeight)
            .contentShape(.rect)
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

/// How many columns and rows the pinned strip uses for the available width.
private struct PinnedGridLayout {
    let columns: Int
    let rows: Int

    static func forWidth(
        _ width: CGFloat, tabCount: Int, itemSize: CGFloat, spacing: CGFloat
    ) -> PinnedGridLayout {
        guard tabCount > 0 else { return PinnedGridLayout(columns: 0, rows: 0) }
        let columns: Int
        if width > 0 {
            columns = max(1, Int((width + spacing) / (itemSize + spacing)))
        } else {
            columns = tabCount
        }
        let rows = Int(ceil(Double(tabCount) / Double(columns)))
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
            .contentShape(.rect)
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
            .onDrag {
                let id = tab.id
                DispatchQueue.main.async { drag.begin(id) }
                return NSItemProvider(object: String(id) as NSString)
            } preview: {
                FavoriteDragPreview(tab: tab)
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

private struct PinnedStripWidthKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
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
    func performDrop(info: DropInfo) -> Bool {
        update(info)
        drag.commit()
        return true
    }

    private func update(_ info: DropInfo) {
        guard let dragging = drag.draggingId, tabId != dragging else {
            drag.updateSlot(nil)
            return
        }
        let slot = info.location.x < tabWidth() / 2 ? slotBefore : slotAfter
        drag.updateSlot(slot)
    }
}

/// Trailing drop zone on the last row for pinning from other sections.
private struct FavoriteStripDropDelegate: DropDelegate {
    let drag: TabDragModel

    func validateDrop(info: DropInfo) -> Bool { drag.draggingId != nil }
    func dropEntered(info: DropInfo) { drag.updateSlot(.endOf(.favorite)) }
    func dropUpdated(info: DropInfo) -> DropProposal? {
        drag.updateSlot(.endOf(.favorite))
        return DropProposal(operation: .move)
    }
    func performDrop(info: DropInfo) -> Bool {
        drag.updateSlot(.endOf(.favorite))
        drag.commit()
        return true
    }
}

/// Floating drag image for a pinned favorite — full 40×40 icon chrome.
private struct FavoriteDragPreview: View {
    let tab: TabItem

    var body: some View {
        TabFavicon(tab: tab)
            .frame(width: 20, height: 20)
            .frame(width: 40, height: 40)
            .background(RoundedRectangle(cornerRadius: 10).fill(.quaternary))
    }
}

extension View {
    func pinnedTabDrag(tab: TabItem, next: Int32?, drag: TabDragModel) -> some View {
        modifier(PinnedTabDrag(tab: tab, next: next, drag: drag))
    }
}
