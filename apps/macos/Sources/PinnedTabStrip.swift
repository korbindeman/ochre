import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Arc-style pinned tabs: spacious favicon icons below the URL bar, always
/// resident in memory (ADR-0032). A user close dismisses the icon only.
struct PinnedTabStrip: View {
    @ObservedObject private var state = BrowserState.shared
    @StateObject private var drag = TabDragModel()
    @Namespace private var selection

    private var app: AppDelegate? { NSApp.delegate as? AppDelegate }

    var body: some View {
        let tabs = state.favoriteTabs
        let dismissed = state.dismissedFavorites

        HStack(spacing: 6) {
            ForEach(Array(tabs.enumerated()), id: \.element.id) { index, tab in
                pinnedButton(
                    tab,
                    next: index + 1 < tabs.count ? tabs[index + 1].id : nil
                )
            }
            stripDropTarget(isEmpty: tabs.isEmpty)
            if !dismissed.isEmpty {
                dismissedMenu(dismissed)
            }
        }
        .padding(.vertical, 2)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private func pinnedButton(_ tab: TabItem, next: Int32?) -> some View {
        PinnedTabButton(
            tab: tab,
            isActive: tab.id == state.activeTabId && !state.isSurfaceOpen,
            selection: selection,
            onSelect: { state.handleTabClick(tab.id) },
            onClose: { app?.closeTab(tab.id) }
        )
        .contextMenu { PinnedTabMenu(tab: tab) }
        .pinnedTabDrag(tab: tab, next: next, drag: drag)
        .transition(SidebarSelection.rowTransition)
    }

    /// Drop target at the strip's trailing edge — pins a dragged tab here and
    /// appends it when the strip already has icons.
    @ViewBuilder
    private func stripDropTarget(isEmpty: Bool) -> some View {
        Color.clear
            .frame(width: isEmpty ? nil : 12, height: 40)
            .frame(maxWidth: isEmpty ? .infinity : 12)
            .contentShape(.rect)
            .overlay(alignment: .leading) {
                if !isEmpty {
                    VerticalInsertionIndicator()
                        .opacity(drag.slot == .endOf(.favorite) ? 1 : 0)
                }
            }
            .onDrop(
                of: [.text],
                delegate: FavoriteStripDropDelegate(isEmpty: isEmpty, drag: drag)
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
                .frame(width: 28, height: 36)
        }
        .menuStyle(.borderlessButton)
        .help("Closed pinned tabs")
    }
}

/// One pinned-tab icon — favicon-centered, close on hover, active pill.
private struct PinnedTabButton: View {
    let tab: TabItem
    let isActive: Bool
    let selection: Namespace.ID
    let onSelect: () -> Void
    let onClose: () -> Void

    @State private var hovering = false

    var body: some View {
        ZStack(alignment: .topTrailing) {
            TabFavicon(tab: tab)
                .frame(width: 20, height: 20)
                .frame(width: 40, height: 36)
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
            if hovering {
                Button(action: onClose) {
                    Image(systemName: "xmark")
                        .font(.system(size: 8, weight: .bold))
                        .frame(width: 14, height: 14)
                }
                .buttonStyle(IconButtonStyle(cornerRadius: 7))
                .offset(x: 4, y: -4)
                .help("Close Pinned Tab")
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
            .simultaneousGesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { _ in drag.armWindowDragSuppression() }
            )
            .onDrag {
                let id = tab.id
                drag.armWindowDragSuppression()
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

/// Trailing (or full-width when empty) drop zone for pinning from other sections.
private struct FavoriteStripDropDelegate: DropDelegate {
    let isEmpty: Bool
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
