import AppKit
import SwiftUI

/// Drag-and-drop for the sidebar — custom gesture-driven, no system drag session
/// (ADR-0035; the order/commit model is ADR-0015).
///
/// The core owns tab order (implicit in its `tabs` order — the sidebar sections
/// are filters over it, ADR-0006). A drag never mutates that live: rows feed a
/// `DragGesture` into one shared `TabDragModel`, which resolves the pointer
/// against row frames reported in a single sidebar-wide coordinate space and
/// commits exactly one `browser_tab_move` on release.
///
/// Why not `onDrag`/`onDrop`: on macOS the system path had no drag-end callback
/// (forcing a mouse-up poller that raced the drop), a mosaic of per-row drop
/// targets with dead zones between them, and an insertion placeholder that
/// mutated layout under the pointer (a resolve→shift→resolve feedback loop).
/// Here every point in the sidebar resolves to a slot, `onEnded` commits
/// synchronously, and all drag visuals live in a non-hit-testable overlay so
/// mid-drag layout never changes.

/// Which sidebar section a tab lives in — the destination a drag resolves to.
/// `favorite` = the pinned icon strip, `looseSaved` = saved with no folder,
/// `open` = the working-set tabs, `folder` = saved inside that folder.
enum TabSection: Equatable {
    case favorite
    case looseSaved
    case open
    case folder(Int32)
}

/// Where a drag will land, as an insertion point. `before` places the tab just
/// above (strip: left of) an anchor tab; `endOf` appends to a section;
/// `ontoFolder` files it into a folder (hovering the folder's header).
enum DropSlot: Equatable {
    case before(Int32)
    case endOf(TabSection)
    case ontoFolder(Int32)
}

// ─── Reported geometry ───────────────────────────────────────────────────────

/// Geometry reported by sidebar views so the resolver and the drag overlay share
/// one picture of the tree. `tab`/`favorite`/`folder`/`newTab` are drop anchors;
/// `favoriteStrip`, `viewport`, and `root` are regions (strip band for horizontal
/// resolution, scroll viewport for edge autoscroll, root for cancel-outside).
enum SidebarDropFrameKind: Equatable {
    case tab(Int32)
    case favorite(Int32)
    case favoriteStrip
    case newTab(looseEmpty: Bool, firstOpen: Int32?)
    case folder(Int32)
    case viewport
    case root
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

/// Coordinate space shared by the whole sidebar tree (declared on the sidebar
/// root): the pinned strip, the tab list, the drag gesture, and the overlay all
/// speak in it.
enum SidebarDropSpace {
    static let name = "sidebarTree"
}

extension View {
    /// Report this row's frame into the drop resolver. `active: false` (the inert
    /// filmstrip panels for non-active spaces, ADR-0043) reports nothing, so the
    /// Y-based resolver only ever sees the one live list.
    @ViewBuilder
    func reportSidebarDropFrame(_ kind: SidebarDropFrameKind, active: Bool = true) -> some View {
        if active {
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
        } else {
            self
        }
    }
}

// ─── Drag state ──────────────────────────────────────────────────────────────

/// High-frequency drag geometry — the live pointer and the reported frames.
/// Split from `TabDragModel` so only the overlay observes it: pointer moves at
/// event rate and must not re-render every row.
@MainActor
final class TabDragGeometry: ObservableObject {
    @Published var pointer: CGPoint = .zero
    @Published var frames: [SidebarDropFrame] = []
}

/// Shared state + state machine for an in-flight tab drag. One instance per
/// sidebar; rows observe `draggingId` (dims the source) and `slot` (folder
/// highlight, empty-strip zone), both low-frequency.
@MainActor
final class TabDragModel: ObservableObject {
    @Published private(set) var draggingId: Int32?
    @Published private(set) var slot: DropSlot?
    let geometry = TabDragGeometry()

    /// The AppKit scroll view hosting the tab list, for edge autoscroll
    /// (grabbed by `ScrollViewProbe`).
    weak var scrollView: NSScrollView?

    /// Live models, weakly held — `--drag-selftest` drives whichever sidebar is
    /// actually laid out (the one with reported frames).
    static let liveModels = NSHashTable<TabDragModel>.weakObjects()
    /// The selftest paces synthetic drags across runloop turns with no physical
    /// mouse down, which the leak-guard watchdog would misread as a stale drag.
    static var watchdogDisabledForTesting = false

    private var escMonitor: Any?
    private var watchdog: Timer?
    private var autoscrollTimer: Timer?
    private var autoscrollStep: CGFloat = 0
    /// Esc cancelled mid-gesture — swallow the rest of that gesture's events.
    private var sessionCancelled = false

    init() {
        Self.liveModels.add(self)
    }

    var isDragging: Bool { draggingId != nil }

    /// Pinning from another section, or reordering a favorite within the strip.
    var canDropOnFavoriteStrip: Bool {
        guard let id = draggingId else { return false }
        let state = BrowserState.shared
        if state.canPinMoreFavorites { return true }
        return state.tabs.first { $0.id == id }?.favorite ?? false
    }

    // ─── Gesture entry points (rows and strip icons feed these) ─────────────

    func dragChanged(_ id: Int32, location: CGPoint) {
        if sessionCancelled { return }
        if draggingId == nil { begin(id) }
        guard draggingId == id else { return }
        geometry.pointer = location
        resolveSlot(at: location)
        updateAutoscroll(for: location)
    }

    func dragEnded(location: CGPoint) {
        defer { sessionCancelled = false }
        guard !sessionCancelled, draggingId != nil else { return }
        geometry.pointer = location
        // Released outside the sidebar entirely — cancel. (The hook a future
        // tear-off-to-window builds on.)
        if let root = geometry.frames.first(where: { $0.kind == .root })?.rect,
            !root.contains(location)
        {
            end()
            return
        }
        resolveSlot(at: location)
        commit()
    }

    /// Abort the in-flight drag without committing (Esc). The active gesture's
    /// remaining events are swallowed until its `onEnded` clears the flag.
    func cancelDrag() {
        guard draggingId != nil else { return }
        sessionCancelled = true
        end()
    }

    /// New frames from the preference pass. Mid-drag this re-aims the slot —
    /// content moves under a stationary pointer during autoscroll.
    func framesChanged(_ frames: [SidebarDropFrame]) {
        geometry.frames = frames
        if draggingId != nil {
            resolveSlot(at: geometry.pointer)
        }
    }

    // ─── Lifecycle ───────────────────────────────────────────────────────────

    private func begin(_ id: Int32) {
        draggingId = id
        slot = nil
        installEscMonitor()
        startWatchdog()
    }

    private func end() {
        stopAutoscroll()
        watchdog?.invalidate()
        watchdog = nil
        removeEscMonitor()
        draggingId = nil
        slot = nil
    }

    private func resolveSlot(at location: CGPoint) {
        guard let id = draggingId else { return }
        updateSlot(SidebarDropResolver.slot(at: location, in: geometry.frames, draggingId: id))
    }

    /// Adopt a candidate slot, suppressing illegal targets (a full strip) and
    /// slots that would leave the tab exactly where it already is — so the
    /// indicator never lies and a drop there is a clean no-op.
    private func updateSlot(_ candidate: DropSlot?) {
        guard let id = draggingId, let candidate else {
            slot = nil
            return
        }
        if targetSection(of: candidate) == .favorite, !canDropOnFavoriteStrip {
            slot = nil
            return
        }
        slot = isNoOp(candidate, for: id) ? nil : candidate
    }

    /// Commit the current `slot` for the dragged tab as a single core move, then
    /// clear the drag. A no-op if there's nothing to move.
    private func commit() {
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
            guard canDropOnFavoriteStrip else { return }
            app?.moveTab(id, favorite: true, pinned: false, folder: nil, before: nil)
        case .endOf(.looseSaved):
            app?.moveTab(id, favorite: false, pinned: true, folder: nil, before: nil)
        case .endOf(.open):
            app?.moveTab(id, favorite: false, pinned: false, folder: nil, before: nil)
        case .endOf(.folder(let f)), .ontoFolder(let f):
            app?.moveTab(id, favorite: false, pinned: true, folder: f, before: nil)
        }
    }

    private func targetSection(of slot: DropSlot) -> TabSection {
        switch slot {
        case .before(let anchor):
            return SidebarDropResolver.section(of: anchor, state: BrowserState.shared)
        case .endOf(let section):
            return section
        case .ontoFolder(let folder):
            return .folder(folder)
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

    // ─── Esc cancel ──────────────────────────────────────────────────────────

    private func installEscMonitor() {
        guard escMonitor == nil else { return }
        escMonitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown]) { [weak self] event in
            guard event.keyCode == 53 else { return event }
            let consumed = MainActor.assumeIsolated { () -> Bool in
                guard let self, self.draggingId != nil else { return false }
                self.cancelDrag()
                return true
            }
            return consumed ? nil : event
        }
    }

    private func removeEscMonitor() {
        if let escMonitor {
            NSEvent.removeMonitor(escMonitor)
        }
        escMonitor = nil
    }

    // ─── Watchdog ────────────────────────────────────────────────────────────

    /// Leak guard only — `DragGesture.onEnded` is the real end-of-drag and
    /// commits synchronously (no race, unlike the old mouse-up poller). This
    /// fires solely when AppKit tears the gesture down without an `onEnded`
    /// (view removed mid-drag) so a phantom drag never stays armed.
    private func startWatchdog() {
        guard !Self.watchdogDisabledForTesting else { return }
        watchdog = Timer.scheduledTimer(withTimeInterval: 0.3, repeats: true) { [weak self] _ in
            Task { @MainActor in
                guard let self, self.draggingId != nil, NSEvent.pressedMouseButtons == 0
                else { return }
                self.end()
            }
        }
    }

    // ─── Edge autoscroll ─────────────────────────────────────────────────────

    private func updateAutoscroll(for location: CGPoint) {
        let zone: CGFloat = 28
        guard
            let viewport = geometry.frames.first(where: { $0.kind == .viewport })?.rect,
            viewport.contains(location)
        else {
            stopAutoscroll()
            return
        }
        if location.y < viewport.minY + zone {
            startAutoscroll(step: -5)
        } else if location.y > viewport.maxY - zone {
            startAutoscroll(step: 5)
        } else {
            stopAutoscroll()
        }
    }

    private func startAutoscroll(step: CGFloat) {
        autoscrollStep = step
        guard autoscrollTimer == nil else { return }
        autoscrollTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 60, repeats: true) {
            [weak self] _ in
            Task { @MainActor in self?.autoscrollTick() }
        }
    }

    private func stopAutoscroll() {
        autoscrollTimer?.invalidate()
        autoscrollTimer = nil
    }

    private func autoscrollTick() {
        guard draggingId != nil, let scrollView, let doc = scrollView.documentView else {
            stopAutoscroll()
            return
        }
        let clip = scrollView.contentView
        let maxOffset = max(0, doc.frame.height - clip.bounds.height)
        let target = min(max(0, clip.bounds.origin.y + autoscrollStep), maxOffset)
        guard target != clip.bounds.origin.y else { return }
        clip.setBoundsOrigin(CGPoint(x: clip.bounds.origin.x, y: target))
        scrollView.reflectScrolledClipView(clip)
        // The rows moved under a stationary pointer; `framesChanged` re-aims the
        // slot once the next layout pass republishes their frames.
    }
}

// ─── Pointer → slot resolution ───────────────────────────────────────────────

@MainActor
enum SidebarDropResolver {
    /// Resolve a pointer (sidebar space) to a drop slot. The strip band resolves
    /// horizontally among favorite icons; everything below resolves vertically
    /// among rows, nearest-row fallback in the gaps — every point in the sidebar
    /// yields *some* slot, so a drop can never silently vanish.
    static func slot(
        at location: CGPoint,
        in frames: [SidebarDropFrame],
        draggingId: Int32
    ) -> DropSlot? {
        if let strip = frames.first(where: { $0.kind == .favoriteStrip })?.rect,
            location.y <= strip.maxY + 4
        {
            // At or above the strip band: favorites territory (above it entirely
            // is nav chrome — no target).
            guard location.y >= strip.minY - 4 else { return nil }
            return stripSlot(at: location, in: frames)
        }
        return listSlot(at: location, in: frames)
    }

    /// Horizontal resolution within the pinned strip's icon grid (row-major,
    /// mirroring store order).
    private static func stripSlot(
        at location: CGPoint,
        in frames: [SidebarDropFrame]
    ) -> DropSlot? {
        let icons: [(id: Int32, rect: CGRect)] = frames.compactMap { frame in
            if case .favorite(let id) = frame.kind { return (id, frame.rect) }
            return nil
        }
        guard !icons.isEmpty else { return .endOf(.favorite) }
        let ordered = icons.sorted { lhs, rhs in
            abs(lhs.rect.minY - rhs.rect.minY) < 1
                ? lhs.rect.minX < rhs.rect.minX
                : lhs.rect.minY < rhs.rect.minY
        }
        // The icon row under the pointer (nearest by vertical center).
        let rowY = ordered.min {
            abs($0.rect.midY - location.y) < abs($1.rect.midY - location.y)
        }!.rect.midY
        let row = ordered.filter { abs($0.rect.midY - rowY) < 1 }
        if let hit = row.first(where: { location.x < $0.rect.midX }) {
            return .before(hit.id)
        }
        // Past the row's last icon: the next row's first icon, or the strip end.
        if let next = ordered.first(where: { $0.rect.midY > rowY + 1 }) {
            return .before(next.id)
        }
        return .endOf(.favorite)
    }

    /// Vertical resolution over the tab tree: a row's top half inserts before it,
    /// its bottom half before its successor (or appends to its section); folder
    /// headers file into the folder; the New Tab row is the saved/open boundary.
    private static func listSlot(
        at location: CGPoint,
        in frames: [SidebarDropFrame]
    ) -> DropSlot? {
        let state = BrowserState.shared
        let ordered = frames.filter { isListAnchor($0.kind) }.sorted { lhs, rhs in
            if lhs.rect.minY == rhs.rect.minY { return lhs.rect.minX < rhs.rect.minX }
            return lhs.rect.minY < rhs.rect.minY
        }
        guard let first = ordered.first, let last = ordered.last else { return .endOf(.open) }
        // Below the last row — the empty scroll area appends to Open.
        if location.y > last.rect.maxY { return .endOf(.open) }
        let hit =
            ordered.first(where: { location.y >= $0.rect.minY && location.y <= $0.rect.maxY })
            ?? (location.y < first.rect.minY
                ? first
                : ordered.min(by: {
                    abs($0.rect.midY - location.y) < abs($1.rect.midY - location.y)
                })!)

        switch hit.kind {
        case .tab(let id):
            if location.y < hit.rect.midY {
                return .before(id)
            }
            if let next = nextTab(after: id, state: state) {
                return .before(next)
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
        default:
            return nil
        }
    }

    private static func isListAnchor(_ kind: SidebarDropFrameKind) -> Bool {
        switch kind {
        case .tab, .newTab, .folder: return true
        case .favorite, .favoriteStrip, .viewport, .root: return false
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

// ─── Drag source modifier ────────────────────────────────────────────────────

/// Makes a tab row or pinned-strip icon a drag source: dims while it is the tab
/// being dragged, and feeds the drag gesture into the shared model. The 5 pt
/// minimum distance keeps plain clicks on `onTapGesture`.
struct SidebarTabDrag: ViewModifier {
    let tab: TabItem
    @ObservedObject var drag: TabDragModel

    func body(content: Content) -> some View {
        content
            .opacity(drag.draggingId == tab.id ? 0.35 : 1)
            .gesture(
                DragGesture(minimumDistance: 5, coordinateSpace: .named(SidebarDropSpace.name))
                    .onChanged { drag.dragChanged(tab.id, location: $0.location) }
                    .onEnded { drag.dragEnded(location: $0.location) }
            )
    }
}

extension View {
    /// `enabled: false` (an inert filmstrip panel, ADR-0043) attaches no drag
    /// gesture, so only the active space's rows are draggable.
    @ViewBuilder
    func sidebarTabDrag(tab: TabItem, drag: TabDragModel, enabled: Bool = true) -> some View {
        if enabled {
            modifier(SidebarTabDrag(tab: tab, drag: drag))
        } else {
            self
        }
    }
}

// ─── Drag visuals (overlay — never in the layout or hit-test tree) ───────────

/// The insertion indicator's geometry, derived from the slot + reported frames.
struct DropIndicatorLine: Equatable {
    var center: CGPoint
    var length: CGFloat
    var isVertical: Bool
}

@MainActor
enum DropIndicator {
    static func line(for slot: DropSlot?, frames: [SidebarDropFrame]) -> DropIndicatorLine? {
        switch slot {
        case nil, .ontoFolder:
            // No line: nothing targeted, or the folder header highlight is the
            // affordance (`FolderRow.isDropTarget`).
            return nil
        case .before(let id):
            guard let (rect, favorite) = anchorRect(of: id, in: frames) else { return nil }
            return favorite ? verticalLine(atX: rect.minX - 3, beside: rect) : horizontalLine(atY: rect.minY - 2, over: rect)
        case .endOf(let section):
            return endLine(for: section, frames: frames)
        }
    }

    private static func endLine(
        for section: TabSection,
        frames: [SidebarDropFrame]
    ) -> DropIndicatorLine? {
        let state = BrowserState.shared
        let members: [TabItem]
        switch section {
        case .favorite: members = state.favoriteTabs
        case .looseSaved: members = state.looseSavedTabs
        case .open: members = state.openTabs
        case .folder(let id): members = state.tabs(inFolder: id)
        }
        if let last = members.last, let (rect, favorite) = anchorRect(of: last.id, in: frames) {
            return favorite ? verticalLine(atX: rect.maxX + 3, beside: rect) : horizontalLine(atY: rect.maxY + 2, over: rect)
        }
        // Empty section: anchor on the New Tab row (saved lands above it, open
        // below). An empty strip highlights its own pin zone — no line.
        guard
            let newTab = frames.first(where: {
                if case .newTab = $0.kind { return true }
                return false
            })?.rect
        else { return nil }
        switch section {
        case .looseSaved: return horizontalLine(atY: newTab.minY - 2, over: newTab)
        case .open: return horizontalLine(atY: newTab.maxY + 2, over: newTab)
        case .favorite, .folder: return nil
        }
    }

    /// The dropped-anchor's frame — a strip icon (vertical indicator) or a list
    /// row (horizontal indicator).
    private static func anchorRect(
        of id: Int32,
        in frames: [SidebarDropFrame]
    ) -> (rect: CGRect, favorite: Bool)? {
        for frame in frames {
            if frame.kind == .favorite(id) { return (frame.rect, true) }
            if frame.kind == .tab(id) { return (frame.rect, false) }
        }
        return nil
    }

    private static func horizontalLine(atY y: CGFloat, over rect: CGRect) -> DropIndicatorLine {
        DropIndicatorLine(
            center: CGPoint(x: rect.midX, y: y),
            length: max(0, rect.width - 8),
            isVertical: false
        )
    }

    private static func verticalLine(atX x: CGFloat, beside rect: CGRect) -> DropIndicatorLine {
        DropIndicatorLine(
            center: CGPoint(x: x, y: rect.midY),
            length: max(0, rect.height - 12),
            isVertical: true
        )
    }
}

/// Full-sidebar overlay carrying every drag visual: the insertion indicator and
/// the ghost of the dragged tab following the pointer. Rendered on the sidebar
/// root so its local coordinates coincide with `SidebarDropSpace`; entirely
/// outside the hit-test tree (CRASH-0001: overlays on gesture trees must never
/// hit-test).
struct SidebarDragOverlay: View {
    @ObservedObject var drag: TabDragModel
    @ObservedObject var geometry: TabDragGeometry
    @ObservedObject private var state = BrowserState.shared

    var body: some View {
        ZStack(alignment: .topLeading) {
            if drag.isDragging {
                if let line = DropIndicator.line(for: drag.slot, frames: geometry.frames) {
                    Capsule()
                        .fill(Color.accentColor)
                        .frame(
                            width: line.isVertical ? 3 : line.length,
                            height: line.isVertical ? line.length : 3
                        )
                        .position(line.center)
                        .transaction { $0.animation = nil }
                }
                if let tab = draggedTab {
                    ghost(for: tab)
                        .offset(x: geometry.pointer.x + 10, y: geometry.pointer.y + 6)
                        .transaction { $0.animation = nil }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .clipped()
        .allowsHitTesting(false)
    }

    private var draggedTab: TabItem? {
        guard let id = drag.draggingId else { return nil }
        return state.tabs.first { $0.id == id }
    }

    @ViewBuilder
    private func ghost(for tab: TabItem) -> some View {
        Group {
            if tab.favorite {
                FavoriteDragGhost(tab: tab)
            } else {
                TabDragGhost(tab: tab)
            }
        }
        .opacity(0.85)
        .fixedSize()
    }
}

/// Floating ghost for a sidebar tab row mid-drag.
private struct TabDragGhost: View {
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
        .frame(minWidth: 140, maxWidth: 200, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary))
    }
}

/// Floating ghost for a pinned favorite mid-drag — full 40×40 icon chrome.
private struct FavoriteDragGhost: View {
    let tab: TabItem

    var body: some View {
        TabFavicon(tab: tab)
            .frame(width: 20, height: 20)
            .frame(width: 40, height: 40)
            .background(RoundedRectangle(cornerRadius: 10).fill(.quaternary))
    }
}

/// Ghost icon marking the pinned strip's pin zone while the strip is empty and a
/// pinnable tab is mid-drag — lit when the drag is aimed at it.
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

// ─── AppKit probes ───────────────────────────────────────────────────────────

/// Grabs the AppKit scroll view enclosing the tab list so the drag model can
/// edge-autoscroll mid-drag (SwiftUI has no scroll-by-offset API on this path).
struct ScrollViewProbe: NSViewRepresentable {
    let drag: TabDragModel

    func makeNSView(context: Context) -> ProbeView {
        ProbeView(drag: drag)
    }

    func updateNSView(_ nsView: ProbeView, context: Context) {
        nsView.drag = drag
    }

    final class ProbeView: NSView {
        weak var drag: TabDragModel?

        init(drag: TabDragModel) {
            self.drag = drag
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError("unused") }

        /// Spans the whole scroll content as a SwiftUI background; a raw NSView
        /// here would win AppKit hit-testing over the hosting view and swallow
        /// every click and gesture on the tab list. Probe only — never a target.
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            drag?.scrollView = enclosingScrollView
        }
    }
}
