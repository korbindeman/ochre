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
    private let state: BrowserState
    private let isPrivate: Bool

    private let background = NSVisualEffectView()
    private lazy var sidebarHost = NSHostingView(rootView: Sidebar(isPrivate: isPrivate))
    private let divider = NSView()
    /// Transparent, wider grab strip on the sidebar's right edge — drag it to
    /// resize. Tracks the divider seam when pinned and the floating card's edge
    /// when revealed; hidden when the sidebar is collapsed.
    private let resizeHandle = SidebarResizeHandle()
    /// Hosts the per-tab CEF views and the empty state, in the content region.
    private let contentArea = NSView()
    private let emptyStateHost: NSHostingView<AnyView>
    private let hotZone = TrackingView()
    private lazy var floatingHost = NSHostingView(rootView: FloatingSidebar(isPrivate: isPrivate))
    /// The History page — a native content-area surface shown over the web
    /// content and dismissed back to it (ADR-0010), never a tab.
    private let historyHost = NSHostingView(rootView: HistoryView())
    /// The Passwords (vault) page — another content-area surface (ADR-0010/0016).
    private let vaultHost = NSHostingView(rootView: VaultView())
    /// The Settings page — native content-area surface (ADR-0010).
    private let settingsHost = NSHostingView(rootView: SettingsView())
    /// Safari-style address bar floating over the page viewport when enabled.
    private lazy var viewportURLBarHost: ViewportURLBarHostingView = {
        let host = ViewportURLBarHostingView(
            rootView: AnyView(URLBar(placement: .viewport).allowsHitTesting(true)))
        return host
    }()

    /// One CEF host view per tab, keyed by tab id. Reconciled against
    /// `BrowserState.tabs`; the active one is shown, the rest hidden.
    private var tabViews: [Int32: CEFContainerView] = [:]
    private var cancellables: Set<AnyCancellable> = []

    /// Seeded from the persisted state so the first layout pass already matches
    /// (the `$isSidebarPinned` observer drops the initial value — it only
    /// animates *changes*).
    private var sidebarPinned: Bool
    /// Bumped at each pin/unpin animation start so a stale completion handler
    /// from an interrupted transition can't clear a newer one.
    private var sidebarTransitionGeneration: UInt = 0
    /// Drives CEF resizes from the content area's *presentation* bounds during
    /// sidebar pin/unpin — at most once per display frame, not every layout pass.
    private var sidebarTransitionTimer: Timer?
    /// Skips redundant Chromium resizes when the animated bounds haven't moved.
    private var lastSyncedCEFSize = NSSize.zero
    /// Whether the floating host is currently shown — mirrors the derived rule
    /// `BrowserState.isFloatingSidebarShown`. Drives the floating card's geometry
    /// in `applyLayout`; kept in step by `syncFloatingSidebar`.
    private var floatingShown = false

    /// A divider drag is in progress — suspends the floating sidebar's hover
    /// dismissal so dragging its edge past the grace margin doesn't snap it shut.
    private var isResizingSidebar = false
    /// Polls the pointer while the sidebar is hover-revealed (see the dismiss
    /// monitor); nil when pinned or collapsed.
    private var revealDismissTimer: Timer?

    /// User-resizable sidebar width, clamped to [min, max] and persisted across
    /// launches. Both the pinned and the floating sidebar render at this width.
    private var sidebarWidth: CGFloat = RootView.loadSidebarWidth()
    private var chromeWidth: CGFloat { sidebarWidth + 1 }  // sidebar + divider

    private static let sidebarWidthKey = BrowserProfile.defaultsKey("sidebarWidth")
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
    /// Matches the sidebar list spring in `BrowserState.reloadStructure` (0.24s).
    private let sidebarPinAnimationDuration = 0.24

    private static func clampSidebarWidth(_ width: CGFloat) -> CGFloat {
        min(maxSidebarWidth, max(minSidebarWidth, width))
    }

    private static func loadSidebarWidth() -> CGFloat {
        let stored = UserDefaults.standard.double(forKey: sidebarWidthKey)
        return stored > 0 ? clampSidebarWidth(CGFloat(stored)) : defaultSidebarWidth
    }

    init(frame frameRect: NSRect, state: BrowserState, isPrivate: Bool) {
        self.state = state
        self.isPrivate = isPrivate
        self.sidebarPinned = state.isSidebarPinned
        emptyStateHost = NSHostingView(
            rootView: isPrivate ? AnyView(PrivateEmptyStateView()) : AnyView(EmptyStateView())
        )
        super.init(frame: frameRect)

        background.material = isPrivate ? .hudWindow : .sidebar
        background.blendingMode = .behindWindow
        background.state = .active

        divider.wantsLayer = true
        divider.layer?.backgroundColor = NSColor.separatorColor.cgColor

        // Content children fill the content area and follow its resizes.
        contentArea.autoresizesSubviews = true
        contentArea.clipsToBounds = true
        // Layer-backed so we can read `presentation()` while AppKit animates the
        // frame and drive CEF resizes from the interpolated size.
        contentArea.wantsLayer = true
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

        // The Passwords surface — same hidden-until-summoned model as History.
        vaultHost.autoresizingMask = [.width, .height]
        vaultHost.frame = contentArea.bounds
        vaultHost.safeAreaRegions = []
        vaultHost.isHidden = true
        contentArea.addSubview(vaultHost)

        // The Settings surface — same hidden-until-summoned model as History.
        settingsHost.autoresizingMask = [.width, .height]
        settingsHost.frame = contentArea.bounds
        settingsHost.safeAreaRegions = []
        settingsHost.isHidden = true
        contentArea.addSubview(settingsHost)

        viewportURLBarHost.autoresizingMask = [.width, .minYMargin]
        viewportURLBarHost.frame = NSRect(
            x: 0, y: 0, width: 1, height: ViewportURLBarMetrics.height)
        viewportURLBarHost.safeAreaRegions = []
        viewportURLBarHost.clipsToBounds = false
        viewportURLBarHost.wantsLayer = true
        viewportURLBarHost.layer?.backgroundColor = NSColor.clear.cgColor
        viewportURLBarHost.isHidden = true
        contentArea.addSubview(viewportURLBarHost)

        // The islands manage their own top insets (traffic-light clearance);
        // without this the hosting views also apply the window's safe area
        // and the content lands twice as low.
        sidebarHost.safeAreaRegions = []
        floatingHost.safeAreaRegions = []
        sidebarHost.wantsLayer = true
        floatingHost.wantsLayer = true
        floatingHost.alphaValue = 0
        floatingHost.isHidden = true

        hotZone.onChange = { [weak self] hovering in
            if hovering {
                self?.state.revealSidebar()
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

    /// Track the core-owned tab model and the sidebar state: prune views for
    /// closed tabs, show the active tab (or the empty state), and keep the floating
    /// sidebar in step with the derived rule (`BrowserState.isFloatingSidebarShown`).
    /// The tab *list* and *active* tab are the core's truth; this view is a
    /// projection of them. Every publisher the rule depends on — pin, hover, active
    /// tab, the two surfaces — re-runs `syncFloatingSidebar`; the method is
    /// idempotent, so redundant calls are free.
    private func observeState() {
        state.$tabs
            .receive(on: RunLoop.main)
            .sink { [weak self] tabs in
                self?.pruneOrphanViews(liveIds: tabs.map(\.id))
                self?.preloadFavoriteTabs()
            }
            .store(in: &cancellables)
        state.$activeSpaceId
            .receive(on: RunLoop.main)
            .sink { [weak self] _ in self?.preloadFavoriteTabs() }
            .store(in: &cancellables)
        state.$activeTabId
            .receive(on: RunLoop.main)
            .sink { [weak self] active in
                self?.showActive(active)
                self?.syncFloatingSidebar()
                self?.syncViewportURLBar()
            }
            .store(in: &cancellables)
        state.$suspendedTabId
            .compactMap { $0 }
            .receive(on: RunLoop.main)
            .sink { [weak self] id in
                self?.detachSuspendedTab(id)
            }
            .store(in: &cancellables)
        state.$isHistoryOpen
            .receive(on: RunLoop.main)
            .sink { [weak self] open in
                self?.setHistoryOpen(open)
                self?.syncFloatingSidebar()
            }
            .store(in: &cancellables)
        state.$isVaultOpen
            .receive(on: RunLoop.main)
            .sink { [weak self] open in
                self?.setVaultOpen(open)
                self?.syncFloatingSidebar()
            }
            .store(in: &cancellables)
        state.$isSettingsOpen
            .receive(on: RunLoop.main)
            .sink { [weak self] open in
                self?.setSettingsOpen(open)
                self?.syncFloatingSidebar()
            }
            .store(in: &cancellables)
        state.$isViewportURLBarEnabled
            .receive(on: RunLoop.main)
            .sink { [weak self] _ in self?.syncViewportURLBar() }
            .store(in: &cancellables)

        Publishers.Merge3(state.$isHistoryOpen, state.$isVaultOpen, state.$isSettingsOpen)
            .receive(on: RunLoop.main)
            .sink { [weak self] _ in self?.syncViewportURLBar() }
            .store(in: &cancellables)
        // Pinning animates the chrome; the floating host follows the derived rule.
        // `dropFirst` skips the seeded value — the initial layout already matches
        // (`sidebarPinned` is seeded at declaration, before the first pass).
        state.$isSidebarPinned
            .dropFirst()
            .receive(on: RunLoop.main)
            .sink { [weak self] pinned in
                self?.setSidebarPinned(pinned)
                self?.syncFloatingSidebar()
            }
            .store(in: &cancellables)
        // Hover reveal is one of the two causes of the floating host; it alone
        // drives the pointer-dismissal monitor (the empty-canvas cause needs none).
        state.$isSidebarRevealed
            .dropFirst()
            .receive(on: RunLoop.main)
            .sink { [weak self] hovered in
                guard let self else { return }
                if hovered {
                    self.startRevealDismissMonitor()
                } else {
                    self.stopRevealDismissMonitor()
                }
                self.syncFloatingSidebar()
            }
            .store(in: &cancellables)
    }

    /// Test hook (P0-1 focus self-test, `--focus-selftest`): the CEF host view of
    /// the active tab, if any. The self-test asserts keyboard focus lands on a
    /// descendant of this view.
    var activeCEFContainer: NSView? {
        guard let id = state.activeTabId else { return nil }
        return tabViews[id]
    }

    /// The pinned sidebar host — used by the focus self-test to click empty chrome.
    var focusTestSidebarHost: NSView? { sidebarHost }

    /// Called before a click dispatches — dismiss transient chrome when the user
    /// presses into the web viewport (not the URL bar pill or sidebar).
    func dismissTransientFocusIfViewportClick(_ event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        guard shouldDismissTransientFocus(forClickAt: point) else { return }
        TransientChrome.dismiss()
    }

    private func shouldDismissTransientFocus(forClickAt point: NSPoint) -> Bool {
        guard contentArea.frame.contains(point) else { return false }
        if !viewportURLBarHost.isHidden {
            let barPoint = viewportURLBarHost.convert(point, from: self)
            if viewportURLBarHost.bounds.contains(barPoint),
                let hit = viewportURLBarHost.hitTest(barPoint)
            {
                return false
            }
        }
        return true
    }

    // ─── Tab views ─────────────────────────────────────────────────────────

    /// Open a new tab loading `url`: build its host view, hand it to the core
    /// (which creates the CEF browser and makes the tab active), and show it.
    func openNewTab(url: String) {
        // Opening a tab dismisses any app-UI surface (History / Passwords) if up.
        if !isPrivate { state.closeSurfaces() }
        let container = CEFContainerView()
        container.autoresizingMask = [.width, .height]
        let bounds = contentArea.bounds
        let inset = viewportChromeInsetForLayout()
        CEFContainerView.viewportChromeInset = inset
        container.clipsToBounds = inset == 0
        container.frame = webContainerFrame(in: bounds, chromeInset: inset)
        // Add before creating so the view is in a window (has a real size) when
        // CEF parents its child view into it.
        contentArea.addSubview(
            container, positioned: .below, relativeTo: viewportURLBarHost)

        let size = cefBrowserSize(for: bounds)
        let id: Int32
        if isPrivate {
            id = url.withCString {
                browser_private_tab_create(
                    state.windowId,
                    Unmanaged.passUnretained(container).toOpaque(),
                    Int32(size.width),
                    Int32(size.height),
                    $0
                )
            }
        } else {
            id = url.withCString {
                browser_tab_create(
                    Unmanaged.passUnretained(container).toOpaque(),
                    Int32(size.width),
                    Int32(size.height),
                    $0
                )
            }
        }
        guard id >= 0 else {
            container.removeFromSuperview()
            NSLog("[swift] browser_tab_create failed")
            return
        }
        tabViews[id] = container
        // The core emitted TabSelected(id) during the call above; make sure the
        // freshly-registered view is the one shown.
        showActive(state.activeTabId)
        syncViewportURLBar()
    }

    /// Close tab `id`: route it through CEF's close flow, then detach its child
    /// view so teardown completes (CEF finishes destroying a browser only once
    /// its host view is released — mirroring the window-close path).
    func closeTab(_ id: Int32) {
        let keepView = !isPrivate && (state.tabs.first { $0.id == id }?.favorite ?? false)
        if isPrivate {
            browser_private_tab_close(state.windowId, id)
        } else {
            browser_tab_close(id)
        }
        guard !keepView else { return }
        if let container = tabViews.removeValue(forKey: id) {
            container.detachCEFView()
            container.removeFromSuperview()
        }
    }

    /// Pre-hydrate every favorite in the active space so they stay loaded
    /// (ADR-0032), including dismissed ones.
    private func preloadFavoriteTabs() {
        guard !isPrivate else { return }
        let favorites = state.tabs.filter { $0.space == state.activeSpaceId && $0.favorite }
        for tab in favorites where tabViews[tab.id] == nil {
            hydrateRestoredTab(tab.id)
        }
        if let active = state.activeTabId {
            for (id, container) in tabViews {
                container.isHidden = (id != active)
            }
        }
    }

    /// Detach every tab's CEF child view — used on window close so all browsers
    /// can complete teardown (which quits the message loop).
    func detachAllCEFViews() {
        for container in tabViews.values {
            container.detachCEFView()
        }
    }

    /// Tear down the CEF host for a tab the core suspended (ADR-0033). The tab
    /// row stays; re-selecting hydrates via `hydrateRestoredTab`.
    private func detachSuspendedTab(_ id: Int32) {
        guard let container = tabViews.removeValue(forKey: id) else { return }
        container.detachCEFView()
        container.removeFromSuperview()
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
        let bounds = contentArea.bounds
        let inset = viewportChromeInsetForLayout()
        CEFContainerView.viewportChromeInset = inset
        container.clipsToBounds = inset == 0
        container.frame = webContainerFrame(in: bounds, chromeInset: inset)
        contentArea.addSubview(container, positioned: .below, relativeTo: historyHost)

        let size = cefBrowserSize(for: bounds)
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
        syncViewportURLBar()
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
            fadeInSurface(historyHost)
        } else {
            historyHost.isHidden = true
        }
    }

    /// Show or hide the Passwords surface over the content region (mirrors
    /// `setHistoryOpen`): raised to the front, reloaded from the core, key focus.
    private func setVaultOpen(_ open: Bool) {
        if open {
            vaultHost.frame = contentArea.bounds
            contentArea.addSubview(vaultHost, positioned: .above, relativeTo: nil)
            vaultHost.isHidden = false
            VaultModel.shared.reload()
            window?.makeFirstResponder(vaultHost)
            fadeInSurface(vaultHost)
        } else {
            vaultHost.isHidden = true
        }
    }

    /// Show or hide the Settings surface over the content region.
    private func setSettingsOpen(_ open: Bool) {
        if open {
            settingsHost.frame = contentArea.bounds
            contentArea.addSubview(settingsHost, positioned: .above, relativeTo: nil)
            settingsHost.isHidden = false
            SettingsModel.shared.reload()
            window?.makeFirstResponder(settingsHost)
            fadeInSurface(settingsHost)
        } else {
            settingsHost.isHidden = true
        }
    }

    /// Fade a content surface in on show, mirroring the floating sidebar's reveal
    /// so History/Passwords don't pop over the web view. A soft enter only; the
    /// paired hide stays instant (the page behind is already composited). Instant
    /// under Reduce Motion (style §7), matching the structure-reload gate.
    private func fadeInSurface(_ view: NSView) {
        guard !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else {
            view.alphaValue = 1
            return
        }
        view.alphaValue = 0
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.12
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            view.animator().alphaValue = 1
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
            x: floatingShown ? inset : -sidebarWidth - 16,
            y: inset,
            width: sidebarWidth,
            height: floatingHeight
        )

        // The grab strip straddles the sidebar's right edge — the divider seam
        // when pinned, the floating card's edge when revealed — and is hidden
        // otherwise so it never swallows clicks over the web content or the
        // reveal hot zone.
        resizeHandle.isHidden = !(sidebarPinned || floatingShown)
        resizeHandle.frame =
            sidebarPinned
            ? NSRect(
                x: chrome - resizeHandleWidth / 2, y: 0,
                width: resizeHandleWidth, height: bounds.height)
            : NSRect(
                x: inset + sidebarWidth - resizeHandleWidth / 2, y: inset,
                width: resizeHandleWidth, height: floatingHeight)

        syncViewportURLBar()
    }

    // ─── Viewport URL bar + web content layout ───────────────────────────────

    private func showsViewportURLBar() -> Bool {
        state.isViewportURLBarEnabled
            && !state.isSurfaceOpen
            && state.activeTabId != nil
            && (state.activeTabId.flatMap { tabViews[$0] != nil } ?? false)
    }

    /// Chrome inset when a web tab is showing with the viewport URL bar enabled.
    private func viewportChromeInset() -> CGFloat {
        showsViewportURLBar() ? ViewportURLBarMetrics.chromeInset : 0
    }

    /// Chrome inset for new/restored tabs before the bar visibility check runs.
    private func viewportChromeInsetForLayout() -> CGFloat {
        state.isViewportURLBarEnabled && !state.isSurfaceOpen
            ? ViewportURLBarMetrics.chromeInset : 0
    }

    /// Frame for CEF host views — fills the area below the viewport URL bar.
    private func webContainerFrame(in bounds: NSRect, chromeInset inset: CGFloat? = nil) -> NSRect {
        let chrome = inset ?? viewportChromeInset()
        return NSRect(x: 0, y: 0, width: bounds.width, height: max(1, bounds.height - chrome))
    }

    /// Size reported to CEF — full content height so the page can scroll behind
    /// the transparent chrome strip.
    private func cefBrowserSize(for bounds: NSRect) -> NSSize {
        NSSize(width: max(1, bounds.width), height: max(1, bounds.height))
    }

    private func syncWebContentFrames(using bounds: NSRect? = nil) {
        let area = bounds ?? contentArea.bounds
        let inset = viewportChromeInset()
        CEFContainerView.viewportChromeInset = inset
        let containerFrame = webContainerFrame(in: area)
        let browserSize = cefBrowserSize(for: area)
        for container in tabViews.values {
            container.clipsToBounds = inset == 0
            container.frame = containerFrame
            container.applyCEFSize(browserSize)
        }
    }

    /// Float the viewport URL bar over the top chrome strip and re-layout CEF.
    private func syncViewportURLBar() {
        let show = showsViewportURLBar()
        viewportURLBarHost.isHidden = !show
        syncWebContentFrames()
        guard show else { return }
        let height = ViewportURLBarMetrics.chromeInset
        viewportURLBarHost.frame = NSRect(
            x: 0,
            y: contentArea.bounds.height - height,
            width: contentArea.bounds.width,
            height: height
        )
        contentArea.addSubview(viewportURLBarHost, positioned: .above, relativeTo: nil)
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
        guard BrowserState.shared.isSidebarRevealed else {
            stopRevealDismissMonitor()
            return
        }
        // Suspend dismissal while resizing the sidebar (the pointer roams past the
        // edge to drag it) or editing the URL inline (the pointer leaves the hover
        // zone to reach the autocomplete results, and collapsing mid-edit would
        // yank the field away). The empty-canvas float needs no such guard — it
        // isn't hover-driven, so this monitor never runs for it.
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

    /// Animate the chrome as the sidebar pins/unpins (the pinned column slides in
    /// or out). The floating host is handled separately by `syncFloatingSidebar`.
    /// CEF tracks the animated content bounds via `syncCEFDuringSidebarTransition`
    /// (presentation-layer size, display-synced) so the page follows the sidebar
    /// without resizing on every layout pass or jerking at the end.
    private func setSidebarPinned(_ pinned: Bool) {
        guard pinned != sidebarPinned else { return }
        if case .sidebarTransition = CEFContainerView.sidebarLayout {
            sidebarTransitionGeneration &+= 1
            finishSidebarTransition()
        }
        CEFContainerView.sidebarLayout = .sidebarTransition
        lastSyncedCEFSize = .zero
        sidebarPinned = pinned
        if NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            finishSidebarTransition()
            applyLayout()
            return
        }
        startSidebarTransitionSync()
        sidebarTransitionGeneration &+= 1
        let generation = sidebarTransitionGeneration
        NSAnimationContext.runAnimationGroup(
            { context in
                context.duration = sidebarPinAnimationDuration
                context.timingFunction = CAMediaTimingFunction(name: .easeOut)
                context.allowsImplicitAnimation = true
                applyLayout()
            },
            completionHandler: { [weak self] in
                MainActor.assumeIsolated {
                    guard let self, self.sidebarTransitionGeneration == generation else { return }
                    self.finishSidebarTransition()
                }
            })
    }

    private func startSidebarTransitionSync() {
        stopSidebarTransitionSync()
        sidebarTransitionTimer = Timer.scheduledTimer(
            withTimeInterval: 1.0 / 60.0, repeats: true
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.syncCEFDuringSidebarTransition() }
        }
        syncCEFDuringSidebarTransition()
    }

    private func stopSidebarTransitionSync() {
        sidebarTransitionTimer?.invalidate()
        sidebarTransitionTimer = nil
    }

    /// The content area's bounds as AppKit is currently *drawing* them — the
    /// model `bounds` jump to the target immediately under implicit animation.
    private func animatedContentSize() -> NSSize {
        if let presentation = contentArea.layer?.presentation() {
            return presentation.frame.size
        }
        return contentArea.bounds.size
    }

    /// Resize CEF to the animated content bounds, throttled to the display-sync
    /// timer and skipping sub-pixel repeats.
    private func syncCEFDuringSidebarTransition() {
        guard case .sidebarTransition = CEFContainerView.sidebarLayout else { return }
        let animatedBounds =
            contentArea.layer?.presentation()?.frame ?? contentArea.bounds
        let browserSize = cefBrowserSize(for: animatedBounds)
        guard browserSize.width >= 1, browserSize.height >= 1 else { return }
        guard
            abs(browserSize.width - lastSyncedCEFSize.width) >= 0.5
                || abs(browserSize.height - lastSyncedCEFSize.height) >= 0.5
        else { return }
        lastSyncedCEFSize = browserSize
        let inset = viewportChromeInset()
        CEFContainerView.viewportChromeInset = inset
        let containerFrame = webContainerFrame(in: animatedBounds)
        for container in tabViews.values {
            container.clipsToBounds = inset == 0
            container.frame = containerFrame
            container.applyCEFSize(browserSize)
        }
    }

    /// End the transition sync loop and hand layout back to the container.
    private func finishSidebarTransition() {
        guard case .sidebarTransition = CEFContainerView.sidebarLayout else { return }
        stopSidebarTransitionSync()
        CEFContainerView.sidebarLayout = .normal
        syncWebContentFrames()
        for container in tabViews.values {
            container.needsLayout = true
            container.layoutSubtreeIfNeeded()
        }
        lastSyncedCEFSize = .zero
    }

    /// Animate the floating sidebar host in/out to match the derived rule
    /// (`BrowserState.isFloatingSidebarShown`): unpinned, and either hover-revealed
    /// or floating over an empty canvas. The single place the host is shown/hidden —
    /// it reads the rule rather than storing a copy, so it can't drift. Idempotent
    /// (guards on the current state), so the several publishers that feed it in
    /// `observeState` can all call it freely.
    private func syncFloatingSidebar() {
        let shown = BrowserState.shared.isFloatingSidebarShown
        guard shown != floatingShown else { return }
        floatingShown = shown
        if shown { floatingHost.isHidden = false }
        NSAnimationContext.runAnimationGroup(
            { context in
                context.duration = 0.16
                context.timingFunction = CAMediaTimingFunction(name: .easeOut)
                context.allowsImplicitAnimation = true
                applyLayout()
                floatingHost.animator().alphaValue = shown ? 1 : 0
            },
            completionHandler: { [weak self] in
                MainActor.assumeIsolated {
                    guard let self, !self.floatingShown else { return }
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
/// A transparent view defaults to `mouseDownCanMoveWindow == true`, which would
/// let a grab on this strip move the window instead of resizing. Return `false`
/// so the seam owns its drag exclusively.
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
