import AppKit
import Combine
import SwiftUI

/// The window's content view — a plain AppKit skeleton.
///
/// AppKit owns the placement of every top-level region and, critically, the CEF
/// tab views' frames. SwiftUI only renders *content* inside hosted islands
/// (sidebar, floating sidebar, empty state); it never positions or restructures
/// around a CEF platform view. Letting SwiftUI manage the CEF view directly
/// proved unreliable: structural changes and frame animations around it stall
/// Chromium's frame pipeline or mis-place the view outright.
///
/// Multi-tab (P1-M1): the content region hosts one `CEFContainerView` per tab
/// plus a native empty state. This view owns those views imperatively —
/// [`openNewTab`] / [`closeTab`] — and reconciles which one is visible against
/// the core's `BrowserState` (the source of truth for the tab list + active tab).
final class RootView: NSView {
    private let background = NSVisualEffectView()
    private let sidebarHost = NSHostingView(rootView: Sidebar())
    private let divider = NSView()
    /// Transparent, wider grab strip on the sidebar's right edge — drag it to
    /// resize. Tracks the divider seam when pinned and the floating card's edge
    /// when revealed; hidden when the sidebar is collapsed.
    private let resizeHandle = SidebarResizeHandle()
    /// Hosts the per-tab CEF views and the empty state, in the content region.
    private let contentArea = NSView()
    private let emptyStateHost = NSHostingView(rootView: EmptyStateView())
    private let hotZone = TrackingView()
    private let floatingHost = NSHostingView(rootView: FloatingSidebar())
    /// The History page — a native content-area surface shown over the web
    /// content and dismissed back to it (ADR-0010), never a tab.
    private let historyHost = NSHostingView(rootView: HistoryView())

    /// One CEF host view per tab, keyed by tab id. Reconciled against
    /// `BrowserState.tabs`; the active one is shown, the rest hidden.
    private var tabViews: [Int32: CEFContainerView] = [:]
    private var cancellables: Set<AnyCancellable> = []

    /// Seeded from the persisted state so the first layout pass already matches
    /// (the `$isSidebarPinned` observer drops the initial value — it only
    /// animates *changes*).
    private var sidebarPinned = BrowserState.shared.isSidebarPinned
    private var sidebarRevealed = false

    /// A divider drag is in progress — suspends the floating sidebar's hover
    /// dismissal so dragging its edge past the grace margin doesn't snap it shut.
    private var isResizingSidebar = false
    /// Polls the pointer while the sidebar is hover-revealed (see the dismiss
    /// monitor); nil when pinned or collapsed.
    private var revealDismissTimer: Timer?

    /// User-resizable sidebar width, clamped to [min, max] and persisted across
    /// launches. Both the pinned and the floating sidebar render at this width.
    private var sidebarWidth: CGFloat = RootView.loadSidebarWidth()
    private var chromeWidth: CGFloat { sidebarWidth + 1 } // sidebar + divider

    private static let sidebarWidthKey = "sidebarWidth"
    private static let defaultSidebarWidth: CGFloat = 224
    private static let minSidebarWidth: CGFloat = 180
    private static let maxSidebarWidth: CGFloat = 400
    /// Width of the invisible drag strip centered on the sidebar's right edge.
    private let resizeHandleWidth: CGFloat = 8
    /// Inset of the floating (hover-revealed) sidebar from the window edges.
    private let floatingSidebarInset: CGFloat = 8

    // Floating-sidebar hover dismissal — how tightly the pointer is polled and
    // how far past the right edge it may roam before the sidebar collapses.
    private let revealPollInterval = 0.05
    private let revealDismissGrace: CGFloat = 48

    private static func clampSidebarWidth(_ width: CGFloat) -> CGFloat {
        min(maxSidebarWidth, max(minSidebarWidth, width))
    }

    private static func loadSidebarWidth() -> CGFloat {
        let stored = UserDefaults.standard.double(forKey: sidebarWidthKey)
        return stored > 0 ? clampSidebarWidth(CGFloat(stored)) : defaultSidebarWidth
    }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)

        background.material = .sidebar
        background.blendingMode = .behindWindow
        background.state = .active

        divider.wantsLayer = true
        divider.layer?.backgroundColor = NSColor.separatorColor.cgColor

        // Content children fill the content area and follow its resizes.
        contentArea.autoresizesSubviews = true
        emptyStateHost.autoresizingMask = [.width, .height]
        emptyStateHost.frame = contentArea.bounds
        emptyStateHost.safeAreaRegions = []
        contentArea.addSubview(emptyStateHost)

        // The History surface fills the content area and stays hidden until
        // summoned; it is raised to the front (over the active tab) on show.
        historyHost.autoresizingMask = [.width, .height]
        historyHost.frame = contentArea.bounds
        historyHost.safeAreaRegions = []
        historyHost.isHidden = true
        contentArea.addSubview(historyHost)

        // The islands manage their own top insets (traffic-light clearance);
        // without this the hosting views also apply the window's safe area
        // and the content lands twice as low.
        sidebarHost.safeAreaRegions = []
        floatingHost.safeAreaRegions = []
        sidebarHost.wantsLayer = true
        floatingHost.wantsLayer = true
        floatingHost.alphaValue = 0
        floatingHost.isHidden = true

        hotZone.onChange = { hovering in
            if hovering {
                BrowserState.shared.revealSidebar()
            }
        }

        // Live-resize the sidebar as its edge is dragged; persist once on release.
        // A drag in progress suspends the floating sidebar's hover dismissal.
        resizeHandle.onDragBegin = { [weak self] in self?.isResizingSidebar = true }
        resizeHandle.onDrag = { [weak self] pointerX in self?.resizeSidebar(to: pointerX) }
        resizeHandle.onDragEnd = { [weak self] in
            self?.isResizingSidebar = false
            self?.persistSidebarWidth()
        }

        // Bottom-up z-order: background, content region, then everything that may
        // sit above the web views. AppKit sibling order is deterministic — this
        // is the whole point of the AppKit skeleton. The resize handle sits on
        // top so it stays grabbable over the floating sidebar card's edge.
        addSubview(background)
        addSubview(contentArea)
        addSubview(sidebarHost)
        addSubview(divider)
        addSubview(hotZone)
        addSubview(floatingHost)
        addSubview(resizeHandle)

        applyLayout()
        observeState()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    /// Track the core-owned tab model: prune views for closed tabs and show the
    /// active tab (or the empty state). The tab *list* and *active* tab are the
    /// core's truth; this view is a projection of them.
    private func observeState() {
        BrowserState.shared.$tabs
            .receive(on: RunLoop.main)
            .sink { [weak self] tabs in self?.pruneOrphanViews(liveIds: tabs.map(\.id)) }
            .store(in: &cancellables)
        BrowserState.shared.$activeTabId
            .receive(on: RunLoop.main)
            .sink { [weak self] active in self?.showActive(active) }
            .store(in: &cancellables)
        BrowserState.shared.$isHistoryOpen
            .receive(on: RunLoop.main)
            .sink { [weak self] open in self?.setHistoryOpen(open) }
            .store(in: &cancellables)
    }

    // ─── Tab views ─────────────────────────────────────────────────────────

    /// Open a new tab loading `url`: build its host view, hand it to the core
    /// (which creates the CEF browser and makes the tab active), and show it.
    func openNewTab(url: String) {
        // Opening a tab dismisses the History surface if it was up.
        BrowserState.shared.closeHistory()
        let container = CEFContainerView()
        container.autoresizingMask = [.width, .height]
        container.frame = contentArea.bounds
        // Add before creating so the view is in a window (has a real size) when
        // CEF parents its child view into it.
        contentArea.addSubview(container)

        let size = contentArea.bounds.size
        let id = url.withCString {
            browser_tab_create(
                Unmanaged.passUnretained(container).toOpaque(),
                Int32(size.width),
                Int32(size.height),
                $0
            )
        }
        guard id >= 0 else {
            container.removeFromSuperview()
            NSLog("[swift] browser_tab_create failed")
            return
        }
        tabViews[id] = container
        // The core emitted TabSelected(id) during the call above; make sure the
        // freshly-registered view is the one shown.
        showActive(BrowserState.shared.activeTabId)
    }

    /// Close tab `id`: route it through CEF's close flow, then detach its child
    /// view so teardown completes (CEF finishes destroying a browser only once
    /// its host view is released — mirroring the window-close path).
    func closeTab(_ id: Int32) {
        browser_tab_close(id)
        if let container = tabViews.removeValue(forKey: id) {
            container.detachCEFView()
            container.removeFromSuperview()
        }
    }

    /// Detach every tab's CEF child view — used on window close so all browsers
    /// can complete teardown (which quits the message loop).
    func detachAllCEFViews() {
        for container in tabViews.values {
            container.detachCEFView()
        }
    }

    /// Drop host views whose tab the core has closed (e.g. a page that closed
    /// itself), so they don't linger over the content region.
    private func pruneOrphanViews(liveIds: [Int32]) {
        let live = Set(liveIds)
        for id in tabViews.keys where !live.contains(id) {
            if let container = tabViews.removeValue(forKey: id) {
                container.detachCEFView()
                container.removeFromSuperview()
            }
        }
    }

    /// Show the active tab's view and hide the rest; reveal the empty state when
    /// there is no active tab. A tab restored from the persisted session has no
    /// view (or browser) until first shown — build + hydrate it here on demand,
    /// so restored tabs the user never revisits never cost a renderer process.
    private func showActive(_ active: Int32?) {
        if let active, tabViews[active] == nil {
            hydrateRestoredTab(active)
        }
        for (id, container) in tabViews {
            container.isHidden = (id != active)
        }
        let hasActiveView = active.map { tabViews[$0] != nil } ?? false
        emptyStateHost.isHidden = hasActiveView
    }

    /// Build the host view for a session-restored tab and ask the core to create
    /// its CEF browser, loading the tab's saved URL (lazy restore, ADR-0013).
    /// Inserted below the History surface so an open History page stays on top,
    /// matching how existing tab views are revealed beneath it.
    private func hydrateRestoredTab(_ id: Int32) {
        let container = CEFContainerView()
        container.autoresizingMask = [.width, .height]
        container.frame = contentArea.bounds
        contentArea.addSubview(container, positioned: .below, relativeTo: historyHost)

        let size = contentArea.bounds.size
        let ok = browser_tab_restore(
            id,
            Unmanaged.passUnretained(container).toOpaque(),
            Int32(size.width),
            Int32(size.height)
        )
        guard ok else {
            container.removeFromSuperview()
            NSLog("[swift] browser_tab_restore(\(id)) failed")
            return
        }
        tabViews[id] = container
    }

    /// Show or hide the History surface over the content region. On show it is
    /// raised to the front so it covers the live CEF view, reloads from the core,
    /// and takes key focus (best-effort) so its search field and Escape work.
    private func setHistoryOpen(_ open: Bool) {
        if open {
            historyHost.frame = contentArea.bounds
            contentArea.addSubview(historyHost, positioned: .above, relativeTo: nil)
            historyHost.isHidden = false
            HistoryModel.shared.reload()
            window?.makeFirstResponder(historyHost)
        } else {
            historyHost.isHidden = true
        }
    }

    // ─── Layout ──────────────────────────────────────────────────────────────

    override func layout() {
        super.layout()
        applyLayout()
    }

    /// Place every region for the current state. Called for window resizes
    /// (unanimated) and from the animated state setters (inside an implicit-
    /// animation context, where frame changes animate).
    private func applyLayout() {
        let bounds = self.bounds
        background.frame = bounds

        let chrome = sidebarPinned ? chromeWidth : 0
        // The pinned sidebar slides out to the left instead of shrinking, so
        // its content never squishes mid-animation.
        sidebarHost.frame = NSRect(
            x: chrome - chromeWidth,
            y: 0,
            width: sidebarWidth,
            height: bounds.height
        )
        divider.frame = NSRect(x: chrome - 1, y: 0, width: 1, height: bounds.height)
        contentArea.frame = NSRect(
            x: chrome,
            y: 0,
            width: max(1, bounds.width - chrome),
            height: bounds.height
        )

        hotZone.frame = NSRect(x: 0, y: 0, width: 12, height: bounds.height)
        let inset = floatingSidebarInset
        let floatingHeight = max(1, bounds.height - inset * 2)
        floatingHost.frame = NSRect(
            x: sidebarRevealed ? inset : -sidebarWidth - 16,
            y: inset,
            width: sidebarWidth,
            height: floatingHeight
        )

        // The grab strip straddles the sidebar's right edge — the divider seam
        // when pinned, the floating card's edge when revealed — and is hidden
        // otherwise so it never swallows clicks over the web content or the
        // reveal hot zone.
        resizeHandle.isHidden = !(sidebarPinned || sidebarRevealed)
        resizeHandle.frame = sidebarPinned
            ? NSRect(x: chrome - resizeHandleWidth / 2, y: 0,
                     width: resizeHandleWidth, height: bounds.height)
            : NSRect(x: inset + sidebarWidth - resizeHandleWidth / 2, y: inset,
                     width: resizeHandleWidth, height: floatingHeight)
    }

    /// Apply a new sidebar width from an edge drag. `pointerX` is in this view's
    /// space; the sidebar's left edge is flush at 0 when pinned and inset when
    /// floating, so subtract it to get the width. Re-lays out immediately (no
    /// animation context) so it tracks the cursor.
    private func resizeSidebar(to pointerX: CGFloat) {
        let leftEdge = sidebarPinned ? 0 : floatingSidebarInset
        let clamped = RootView.clampSidebarWidth(pointerX - leftEdge)
        guard clamped != sidebarWidth else { return }
        sidebarWidth = clamped
        applyLayout()
    }

    private func persistSidebarWidth() {
        UserDefaults.standard.set(Double(sidebarWidth), forKey: RootView.sidebarWidthKey)
    }

    // ─── Floating-sidebar hover dismissal ──────────────────────────────────
    // While the sidebar is hover-revealed, poll the pointer and collapse it the
    // moment it leaves the hover zone (the sidebar's width plus a grace margin
    // past its right edge). An in-progress resize drag suspends this so dragging
    // the edge out past the margin can't collapse the sidebar.

    private func startRevealDismissMonitor() {
        stopRevealDismissMonitor()
        revealDismissTimer = Timer.scheduledTimer(
            withTimeInterval: revealPollInterval, repeats: true
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.tickRevealDismiss() }
        }
    }

    private func tickRevealDismiss() {
        guard sidebarRevealed else { stopRevealDismissMonitor(); return }
        // Suspend dismissal while resizing, or while editing the URL inline — the
        // pointer leaves the hover zone to reach the results, and collapsing the
        // sidebar mid-edit would yank the field away.
        if isResizingSidebar || URLBarModel.shared.isEditing { return }
        guard let window else { return }

        if !revealHoverZone(in: window).contains(NSEvent.mouseLocation) {
            stopRevealDismissMonitor()
            BrowserState.shared.isSidebarRevealed = false
        }
    }

    private func stopRevealDismissMonitor() {
        revealDismissTimer?.invalidate()
        revealDismissTimer = nil
    }

    /// Screen-space region that keeps the floating sidebar revealed: the window's
    /// left edge out past the sidebar's right edge by `revealDismissGrace`, full
    /// height. Scales with the (resizable) sidebar width.
    private func revealHoverZone(in window: NSWindow) -> NSRect {
        let frame = window.frame
        let width = floatingSidebarInset + sidebarWidth + revealDismissGrace
        return NSRect(x: frame.minX, y: frame.minY, width: width, height: frame.height)
    }

    func setSidebarPinned(_ pinned: Bool) {
        guard pinned != sidebarPinned else { return }
        sidebarPinned = pinned
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.28
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            context.allowsImplicitAnimation = true
            applyLayout()
        }
    }

    func setSidebarRevealed(_ revealed: Bool) {
        guard revealed != sidebarRevealed else { return }
        sidebarRevealed = revealed
        if revealed {
            floatingHost.isHidden = false
            startRevealDismissMonitor()
        } else {
            stopRevealDismissMonitor()
        }
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = 0.24
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            context.allowsImplicitAnimation = true
            applyLayout()
            floatingHost.animator().alphaValue = revealed ? 1 : 0
        }, completionHandler: { [weak self] in
            MainActor.assumeIsolated {
                guard let self, !self.sidebarRevealed else { return }
                self.floatingHost.isHidden = true
            }
        })
    }
}

/// The draggable seam between the sidebar and the content region. It shows a
/// left-right resize cursor and reports the pointer's x (in its superview's
/// space) as the drag progresses; `RootView` turns that into the new width.
///
/// Overriding `mouseDown` is what claims the drag — the default forwards it up the
/// responder chain, so without it the follow-up `mouseDragged`/`mouseUp` never
/// arrive here. `acceptsFirstMouse` lets a resize begin even when the window was
/// inactive, matching native split-view behaviour.
///
/// The window is movable by its background (so the sidebar body drags it,
/// Arc-style), and AppKit gates that on the hit view's `mouseDownCanMoveWindow`.
/// A transparent view defaults to `true`, which would let a grab on this strip
/// move the window instead of resizing. Return `false` so the seam owns its drag
/// exclusively — an exclusive resize strip carved out of the draggable sidebar.
final class SidebarResizeHandle: NSView {
    var onDragBegin: (() -> Void)?
    var onDrag: ((CGFloat) -> Void)?
    var onDragEnd: (() -> Void)?

    override var mouseDownCanMoveWindow: Bool { false }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .resizeLeftRight)
    }

    override func mouseDown(with event: NSEvent) {
        NSCursor.resizeLeftRight.set()
        onDragBegin?()
    }

    override func mouseDragged(with event: NSEvent) {
        guard let parent = superview else { return }
        NSCursor.resizeLeftRight.set()
        onDrag?(parent.convert(event.locationInWindow, from: nil).x)
    }

    override func mouseUp(with event: NSEvent) {
        onDragEnd?()
    }
}
