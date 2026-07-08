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
/// live drop target the placeholder follows. One instance is owned by
/// `SpaceContents` and passed to every row, folder header, and boundary zone.
@MainActor
final class TabDragModel: ObservableObject {
    @Published var draggingId: Int32?
    @Published var slot: DropSlot?

    private var dragMonitor: Timer?
    private var isCommitting = false

    var isDragging: Bool { draggingId != nil }

    func begin(_ id: Int32) {
        draggingId = id
        slot = nil
        isCommitting = false
        startDragMonitor()
    }

    func end() {
        dragMonitor?.invalidate()
        dragMonitor = nil
        draggingId = nil
        slot = nil
        isCommitting = false
    }

    /// SwiftUI has no drag-cancel callback; poll for mouse-up so a drop outside
    /// any target still clears the dimmed source row and placeholder.
    private func startDragMonitor() {
        dragMonitor?.invalidate()
        dragMonitor = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] _ in
            Task { @MainActor in
                guard
                    let self,
                    self.draggingId != nil,
                    !self.isCommitting,
                    NSEvent.pressedMouseButtons == 0
                else { return }
                // Defer one turn so `performDrop` can commit before we clear state.
                DispatchQueue.main.async { [weak self] in
                    guard let self, self.draggingId != nil, !self.isCommitting else { return }
                    self.end()
                }
            }
        }
    }

    /// Update the visible insertion point, suppressing slots that would leave the
    /// tab exactly where it already is. Keeping this in the shared model makes row,
    /// boundary, folder, pinned-strip, and overlay delegates agree, and avoids a
    /// pointless structural snapshot/write on drop.
    func updateSlot(_ candidate: DropSlot?) {
        guard let id = draggingId, let candidate else {
            slot = nil
            return
        }
        slot = isNoOp(candidate, for: id) ? nil : candidate
    }

    /// Commit the current `slot` for the dragged tab as a single core move, then
    /// clear the drag. A no-op if there's nothing to move.
    func commit() {
        isCommitting = true
        defer { end() }
        guard let id = draggingId, let slot, !isNoOp(slot, for: id) else { return }
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

    private func isNoOp(_ slot: DropSlot, for id: Int32) -> Bool {
        let state = BrowserState.shared
        let current = SidebarDropResolver.section(of: id, state: state)
        switch slot {
        case .before(id):
            return true
        case .before(let anchor):
            return SidebarDropResolver.section(of: anchor, state: state) == current
                && SidebarDropResolver.nextTab(after: id, state: state) == anchor
        case .endOf(let section):
            guard section == current else { return false }
            return SidebarDropResolver.nextTab(after: id, state: state) == nil
        case .ontoFolder(let folder):
            return current == .folder(folder)
                && SidebarDropResolver.nextTab(after: id, state: state) == nil
        }
    }
}

/// Ghost row shown where a sidebar tab will land — matches `TabRow` chrome.
struct TabDropPlaceholder: View {
    var body: some View {
        HStack(spacing: 8) {
            RoundedRectangle(cornerRadius: 4)
                .fill(Color.accentColor.opacity(0.28))
                .frame(width: 16, height: 16)
            RoundedRectangle(cornerRadius: 3)
                .fill(Color.accentColor.opacity(0.18))
                .frame(height: 10)
                .frame(maxWidth: .infinity, alignment: .leading)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 9)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background {
            RoundedRectangle(cornerRadius: 8)
                .fill(Color.accentColor.opacity(0.14))
            RoundedRectangle(cornerRadius: 8)
                .strokeBorder(Color.accentColor.opacity(0.55), lineWidth: 1.5)
        }
        .allowsHitTesting(false)
        .transaction { $0.animation = nil }
    }
}

// ─── Favorite → sidebar drop surface ─────────────────────────────────────────

/// Geometry reported by sidebar rows so a full-tree overlay can resolve drops
/// from the pinned strip (drags that start outside the `ScrollView` subtree).
enum SidebarDropFrameKind: Equatable {
    case tab(Int32)
    case newTab(looseEmpty: Bool, firstOpen: Int32?)
    case folder(Int32)
}

struct SidebarDropFrame: Equatable {
    var kind: SidebarDropFrameKind
    var rect: CGRect
}

struct SidebarDropFramesKey: PreferenceKey {
    static var defaultValue: [SidebarDropFrame] = []
    static func reduce(value: inout [SidebarDropFrame], nextValue: () -> [SidebarDropFrame]) {
        value.append(contentsOf: nextValue())
    }
}

/// Coordinate space shared by the sidebar tree and the favorite-unpin overlay.
enum SidebarDropSpace {
    static let name = "sidebarTree"
}

extension View {
    func reportSidebarDropFrame(_ kind: SidebarDropFrameKind) -> some View {
        background(
            GeometryReader { geo in
                Color.clear.preference(
                    key: SidebarDropFramesKey.self,
                    value: [
                        SidebarDropFrame(
                            kind: kind,
                            rect: geo.frame(in: .named(SidebarDropSpace.name))
                        )
                    ]
                )
            }
        )
    }
}

@MainActor
enum SidebarDropResolver {
    static func slot(
        at location: CGPoint,
        in frames: [SidebarDropFrame],
        draggingId: Int32
    ) -> DropSlot? {
        let state = BrowserState.shared
        let ordered = frames.sorted { lhs, rhs in
            if lhs.rect.minY == rhs.rect.minY { return lhs.rect.minX < rhs.rect.minX }
            return lhs.rect.minY < rhs.rect.minY
        }
        guard let first = ordered.first, let last = ordered.last else { return .endOf(.open) }

        // The favorite-unpin overlay spans the whole scroll tree, including empty
        // space below the rows. Treat that bottom area as "Open end" instead of
        // snapping to the nearest saved/folder row, which made unpinning feel
        // random in sparse sidebars.
        if location.y > last.rect.maxY { return .endOf(.open) }
        let hit =
            ordered.first(where: { $0.rect.contains(location) })
            ?? (location.y < first.rect.minY
            ? first
            : ordered.min(by: {
                abs($0.rect.midY - location.y) < abs($1.rect.midY - location.y)
            }))!

        switch hit.kind {
        case .tab(let id):
            guard id != draggingId else { return nil }
            if location.y < hit.rect.midY {
                let slot = DropSlot.before(id)
                return slot == .before(draggingId) ? nil : slot
            }
            if let next = nextTab(after: id, state: state) {
                let slot = DropSlot.before(next)
                return slot == .before(draggingId) ? nil : slot
            }
            return .endOf(section(of: id, state: state))
        case .newTab(let looseEmpty, let firstOpen):
            if location.y < hit.rect.midY {
                return looseEmpty ? .endOf(.looseSaved) : nil
            }
            if let firstOpen {
                return .before(firstOpen)
            }
            return .endOf(.open)
        case .folder(let folderId):
            return .ontoFolder(folderId)
        }
    }

    static func section(of tabId: Int32, state: BrowserState) -> TabSection {
        guard let tab = state.tabs.first(where: { $0.id == tabId }) else { return .open }
        if tab.favorite { return .favorite }
        if let folder = tab.folder { return .folder(folder) }
        if tab.pinned { return .looseSaved }
        return .open
    }

    static func nextTab(after tabId: Int32, state: BrowserState) -> Int32? {
        guard state.tabs.first(where: { $0.id == tabId }) != nil else { return nil }
        let section = section(of: tabId, state: state)
        let peers: [TabItem]
        switch section {
        case .favorite:
            peers = state.favoriteTabs
        case .looseSaved:
            peers = state.looseSavedTabs
        case .open:
            peers = state.openTabs
        case .folder(let folderId):
            peers = state.tabs(inFolder: folderId)
        }
        guard let index = peers.firstIndex(where: { $0.id == tabId }),
            index + 1 < peers.count
        else { return nil }
        return peers[index + 1].id
    }
}

/// Full-tree drop catcher while a favorite is mid-drag — `ScrollView` row targets
/// do not reliably receive drags that start in the pinned strip above them.
struct FavoriteSidebarDropLayer: View {
    @ObservedObject var drag: TabDragModel
    let frames: [SidebarDropFrame]

    var body: some View {
        Color.clear
            .contentShape(.rect)
            .onDrop(
                of: [.text],
                delegate: FavoriteSidebarDropDelegate(drag: drag, frames: { frames })
            )
    }
}

private struct FavoriteSidebarDropDelegate: DropDelegate {
    let drag: TabDragModel
    let frames: () -> [SidebarDropFrame]

    func validateDrop(info: DropInfo) -> Bool {
        guard let id = drag.draggingId else { return false }
        return BrowserState.shared.tabs.first(where: { $0.id == id })?.favorite ?? false
    }

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
        guard let dragging = drag.draggingId else { return }
        drag.updateSlot(
            SidebarDropResolver.slot(
                at: info.location,
                in: frames(),
                draggingId: dragging
            )
        )
    }
}

/// Ghost icon shown where a pinned favorite will land — matches `PinnedTabButton`.
struct FavoriteDropPlaceholder: View {
    var body: some View {
        RoundedRectangle(cornerRadius: 10)
            .fill(Color.accentColor.opacity(0.14))
            .overlay {
                RoundedRectangle(cornerRadius: 10)
                    .strokeBorder(Color.accentColor.opacity(0.55), lineWidth: 1.5)
            }
            .overlay {
                RoundedRectangle(cornerRadius: 4)
                    .fill(Color.accentColor.opacity(0.28))
                    .frame(width: 20, height: 20)
            }
            .allowsHitTesting(false)
            .transaction { $0.animation = nil }
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
/// it is the one being dragged; the matching placeholder is rendered in the list
/// layout by `SpaceContents.sectionRows`.
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
            .onDrag {
                let id = tab.id
                DispatchQueue.main.async { drag.begin(id) }
                return NSItemProvider(object: String(id) as NSString)
            } preview: {
                TabDragPreview(tab: tab)
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

    func performDrop(info: DropInfo) -> Bool {
        update(info)
        drag.commit()
        return true
    }

    /// Resolve the pointer position to a slot, suppressing the ones that would
    /// leave the dragged tab exactly where it is (hovering its own row, or the
    /// gap directly above it). Slot is not cleared on `dropExited` — the next
    /// row's `dropEntered` updates it without a flicker gap.
    private func update(_ info: DropInfo) {
        guard let dragging = drag.draggingId, tabId != dragging else {
            drag.updateSlot(nil)
            return
        }
        let slot = info.location.y < rowHeight() / 2 ? slotAbove : slotBelow
        drag.updateSlot(slot)
    }
}

// ─── Folder headers ──────────────────────────────────────────────────────────

/// Dropping a tab on a folder header files it into that folder (auto-pinned).
struct FolderDropDelegate: DropDelegate {
    let folderId: Int32
    let drag: TabDragModel

    func validateDrop(info: DropInfo) -> Bool { drag.draggingId != nil }
    func dropEntered(info: DropInfo) { drag.updateSlot(.ontoFolder(folderId)) }
    func dropUpdated(info: DropInfo) -> DropProposal? {
        drag.updateSlot(.ontoFolder(folderId))
        return DropProposal(operation: .move)
    }

    func performDrop(info: DropInfo) -> Bool {
        drag.updateSlot(.ontoFolder(folderId))
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
    func dropEntered(info: DropInfo) { apply(info) }
    func dropUpdated(info: DropInfo) -> DropProposal? {
        apply(info)
        return DropProposal(operation: .move)
    }

    func performDrop(info: DropInfo) -> Bool {
        apply(info)
        drag.commit()
        return true
    }

    private func apply(_ info: DropInfo) {
        drag.updateSlot(slot(info))
    }
}

/// Floating drag image for a sidebar tab row.
private struct TabDragPreview: View {
    let tab: TabItem

    var body: some View {
        HStack(spacing: 8) {
            TabFavicon(tab: tab)
                .frame(width: 16, height: 16)
            Text(tab.displayTitle)
                .lineLimit(1)
                .font(.system(size: 13, weight: .medium))
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 9)
        .frame(minWidth: 180, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary))
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
