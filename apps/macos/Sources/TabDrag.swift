import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Drag-and-drop reordering for the sidebar tab list.
///
/// The core owns tab order (implicit in its `tabs` order — the sidebar sections
/// are filters over it, ADR-0006). A drag never mutates that live: it tracks a
/// single drop *slot* locally and commits exactly one `browser_tab_move` on drop
/// (the core persists on every change, so live reordering would thrash the
/// session file). The resulting `StructureChanged` snapshot re-renders the list,
/// spring-animated by `BrowserState.reloadStructure`.

/// Which sidebar section a tab lives in — the destination a drag resolves to.
/// Mirrors the sections `SpaceContents` renders: `looseSaved` = saved with no
/// folder, `open` = the working-set tabs, `folder` = saved inside that folder.
enum TabSection: Equatable {
    case favorite
    case looseSaved
    case open
    case folder(Int32)
}

/// Where a drag will land, as an insertion point. `before` places the tab just
/// above an anchor tab; `endOf` appends to a section; `ontoFolder` files it into
/// a folder (dropped on the folder's header).
enum DropSlot: Equatable {
    case before(Int32)
    case endOf(TabSection)
    case ontoFolder(Int32)
}

/// Shared, observable state for an in-flight tab drag: `draggingId` is the tab
/// being dragged (dims its row, suppresses self-targeting) and `slot` is the
/// live drop target the insertion indicator follows. One instance is owned by
/// `SpaceContents` and passed to every row, folder header, and boundary zone.
@MainActor
final class TabDragModel: ObservableObject {
    @Published var draggingId: Int32?
    @Published var slot: DropSlot?

    func begin(_ id: Int32) {
        draggingId = id
        slot = nil
    }

    func end() {
        draggingId = nil
        slot = nil
    }

    /// Commit the current `slot` for the dragged tab as a single core move, then
    /// clear the drag. A no-op if there's nothing to move.
    func commit() {
        defer { end() }
        guard let id = draggingId, let slot else { return }
        let app = NSApp.delegate as? AppDelegate
        switch slot {
        case .before(let anchor):
            guard anchor != id,
                let t = BrowserState.shared.tabs.first(where: { $0.id == anchor })
            else { return }
            app?.moveTab(
                id,
                favorite: t.favorite,
                pinned: t.pinned,
                folder: t.folder,
                before: anchor
            )
        case .endOf(.favorite):
            app?.moveTab(id, favorite: true, pinned: false, folder: nil, before: nil)
        case .endOf(.looseSaved):
            app?.moveTab(id, favorite: false, pinned: true, folder: nil, before: nil)
        case .endOf(.open):
            app?.moveTab(id, favorite: false, pinned: false, folder: nil, before: nil)
        case .endOf(.folder(let f)), .ontoFolder(let f):
            app?.moveTab(id, favorite: false, pinned: true, folder: f, before: nil)
        }
    }
}

/// The thin accent line drawn in the gap where a dropped tab will land.
struct InsertionIndicator: View {
    var body: some View {
        Capsule()
            .fill(Color.accentColor)
            .frame(height: 2.5)
            .padding(.horizontal, 8)
    }
}

/// Row height, measured so a drop delegate can split a row into an above/below
/// half for the insertion point.
struct RowHeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 36
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = nextValue() }
}

// ─── Tab rows ────────────────────────────────────────────────────────────────

/// Drag/drop wiring for one tab row: a drag source, and a drop target whose top
/// half inserts before this tab and whose bottom half inserts before the next
/// sibling (or appends to the section when this row is last). Dims the row while
/// it is the one being dragged and draws the insertion indicator at the matching
/// edge.
struct TabRowDrag: ViewModifier {
    let tab: TabItem
    let section: TabSection
    /// The tab after this one within the same section, or nil if it is last.
    let next: Int32?
    @ObservedObject var drag: TabDragModel

    @State private var height: CGFloat = 36

    private var slotAbove: DropSlot { .before(tab.id) }
    private var slotBelow: DropSlot { next.map { .before($0) } ?? .endOf(section) }

    func body(content: Content) -> some View {
        content
            .opacity(drag.draggingId == tab.id ? 0.35 : 1)
            .background(
                GeometryReader { geo in
                    Color.clear.preference(key: RowHeightKey.self, value: geo.size.height)
                }
            )
            .onPreferenceChange(RowHeightKey.self) { height = $0 }
            .overlay(alignment: .top) {
                InsertionIndicator().opacity(drag.slot == slotAbove ? 1 : 0)
            }
            .overlay(alignment: .bottom) {
                InsertionIndicator()
                    .opacity(next == nil && drag.slot == .endOf(section) ? 1 : 0)
            }
            .onDrag {
                // Defer marking the drag so the floating preview is snapshotted at
                // full opacity — the dim then applies only to the in-place row.
                let id = tab.id
                DispatchQueue.main.async { drag.begin(id) }
                return NSItemProvider(object: String(id) as NSString)
            }
            .onDrop(
                of: [.text],
                delegate: TabRowDropDelegate(
                    tabId: tab.id,
                    slotAbove: slotAbove,
                    slotBelow: slotBelow,
                    rowHeight: { height },
                    drag: drag
                )
            )
    }
}

private struct TabRowDropDelegate: DropDelegate {
    let tabId: Int32
    let slotAbove: DropSlot
    let slotBelow: DropSlot
    let rowHeight: () -> CGFloat
    let drag: TabDragModel

    func validateDrop(info: DropInfo) -> Bool { drag.draggingId != nil }
    func dropEntered(info: DropInfo) { update(info) }
    func dropUpdated(info: DropInfo) -> DropProposal? {
        update(info)
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) {
        if drag.slot == slotAbove || drag.slot == slotBelow { drag.slot = nil }
    }

    func performDrop(info: DropInfo) -> Bool {
        update(info)
        drag.commit()
        return true
    }

    /// Resolve the pointer position to a slot, suppressing the ones that would
    /// leave the dragged tab exactly where it is (hovering its own row, or the
    /// gap directly above it).
    private func update(_ info: DropInfo) {
        guard let dragging = drag.draggingId, tabId != dragging else {
            drag.slot = nil
            return
        }
        let slot = info.location.y < rowHeight() / 2 ? slotAbove : slotBelow
        drag.slot = slot == .before(dragging) ? nil : slot
    }
}

// ─── Folder headers ──────────────────────────────────────────────────────────

/// Dropping a tab on a folder header files it into that folder (auto-pinned).
struct FolderDropDelegate: DropDelegate {
    let folderId: Int32
    let drag: TabDragModel

    func validateDrop(info: DropInfo) -> Bool { drag.draggingId != nil }
    func dropEntered(info: DropInfo) { drag.slot = .ontoFolder(folderId) }
    func dropUpdated(info: DropInfo) -> DropProposal? {
        drag.slot = .ontoFolder(folderId)
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) {
        if drag.slot == .ontoFolder(folderId) { drag.slot = nil }
    }

    func performDrop(info: DropInfo) -> Bool {
        drag.slot = .ontoFolder(folderId)
        drag.commit()
        return true
    }
}

// ─── The saved / open boundary ───────────────────────────────────────────────

/// The New Tab row sits between the saved section and open tabs, so it doubles as
/// the drop boundary and covers the two gaps no tab row can: an *empty* loose-
/// saved section (top half → save) and an *empty* open section (bottom half →
/// open). When those sections have rows, the rows own their own gaps and this
/// row's halves defer to them.
struct BoundaryDropDelegate: DropDelegate {
    /// Only when the loose-saved section has no rows to aim at is the top half a
    /// save target — otherwise the last loose row owns that gap.
    let looseEmpty: Bool
    /// The first open tab, so moving to open lands at the top of the section.
    let firstOpen: Int32?
    let rowHeight: () -> CGFloat
    let drag: TabDragModel

    private func slot(_ info: DropInfo) -> DropSlot? {
        if info.location.y < rowHeight() / 2 {
            return looseEmpty ? .endOf(.looseSaved) : nil
        }
        return firstOpen.map { .before($0) } ?? .endOf(.open)
    }

    func validateDrop(info: DropInfo) -> Bool { drag.draggingId != nil }
    func dropEntered(info: DropInfo) { drag.slot = slot(info) }
    func dropUpdated(info: DropInfo) -> DropProposal? {
        drag.slot = slot(info)
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) { drag.slot = nil }
    func performDrop(info: DropInfo) -> Bool {
        drag.slot = slot(info)
        drag.commit()
        return true
    }
}

/// Drop target at the top of the sidebar tree while a pinned tab is being dragged
/// out — lands in the first section or the open list.
struct FavoriteOutboundDropDelegate: DropDelegate {
    let slot: DropSlot
    let drag: TabDragModel

    func validateDrop(info: DropInfo) -> Bool { drag.draggingId != nil }
    func dropEntered(info: DropInfo) { drag.slot = slot }
    func dropUpdated(info: DropInfo) -> DropProposal? {
        drag.slot = slot
        return DropProposal(operation: .move)
    }
    func dropExited(info: DropInfo) {
        if drag.slot == slot { drag.slot = nil }
    }
    func performDrop(info: DropInfo) -> Bool {
        drag.slot = slot
        drag.commit()
        return true
    }
}

extension View {
    /// Make a tab row a drag source and reorder drop target. `next` is the id of
    /// the following tab in the same section (nil if this row is last).
    func tabRowDrag(
        tab: TabItem,
        section: TabSection,
        next: Int32?,
        drag: TabDragModel
    ) -> some View {
        modifier(TabRowDrag(tab: tab, section: section, next: next, drag: drag))
    }
}
