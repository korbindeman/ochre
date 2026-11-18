import AppKit
import SwiftUI

/// One tab, mirrored from the core's tree over the event stream (ADR-0003/0005).
struct TabItem: Identifiable, Equatable {
    let id: Int32
    var space: Int32
    /// The folder this tab lives in (saved tabs only), or nil for a loose tab.
    var folder: Int32?
    /// Pinned (persistent in this Space) vs. open (working set). Distinct from
    /// `favorite`, which is global and rendered in the icon strip.
    var pinned: Bool
    /// Arc-style Favorites icon strip below the URL bar (ADR-0032). Permanent -
    /// close resets temporary navigation and deselects; the icon stays.
    var favorite: Bool
    /// Durable identity of a pinned tab/favorite. `url` remains the live page
    /// and may differ until close or an explicit replacement (ADR-0056).
    var savedURL: String?
    var title: String = ""
    var url: String = ""
    var isLoading: Bool = false
    /// History enablement, streamed from the core, driving the sidebar's
    /// back/forward controls (disabled when there's nowhere to go).
    var canGoBack: Bool = false
    var canGoForward: Bool = false
    /// Bumped when the engine captures a fresh favicon, to retrigger the load.
    var faviconVersion: Int = 0
    /// Sampled page theme/background color for viewport URL-bar tinting.
    var themeColor: String = ""

    var host: String { URLComponents(string: url)?.host ?? "" }
    var hasTemporaryPinnedNavigation: Bool {
        guard let savedURL else { return false }
        return savedURL != url
    }

    var displayTitle: String {
        if !title.isEmpty { return title }
        if !host.isEmpty { return host }
        return "New Tab"
    }
}

/// A space - a switchable set of tabs (Pinned + Open sections).
/// `icon` is the emoji shown in the space switcher.
struct SpaceItem: Identifiable, Equatable {
    let id: Int32
    var name: String
    var icon: String
    var pinnedExpanded: Bool
}

/// A folder inside a Space's Pinned section; holds pinned tabs and subfolders.
struct FolderItem: Identifiable, Equatable {
    let id: Int32
    var space: Int32
    var name: String
    /// `nil` at the space root; set when nested inside another folder.
    var parent: Int32?
    /// Sidebar expand/collapse state, owned by the core so it persists across
    /// launches and space switches (ADR-0039). `false` = collapsed.
    var expanded: Bool
}

/// Observable UI state, a projection of the core-owned tree. Structure (spaces /
/// folders / tab membership / active) is rebuilt wholesale from a JSON snapshot
/// on `StructureChanged`; hot per-tab content is patched in place (ADR-0005).
@MainActor
final class BrowserState: ObservableObject {
    static let normal = BrowserState(windowId: 0, isPrivate: false)
    static var current: BrowserState = normal
    private(set) static var registry: [Int32: BrowserState] = [0: normal]

    /// The focused window's state (most UI reads this).
    static var shared: BrowserState { current }

    static func register(windowId: Int32, isPrivate: Bool) -> BrowserState {
        let state = BrowserState(windowId: windowId, isPrivate: isPrivate)
        registry[windowId] = state
        return state
    }

    static func unregister(windowId: Int32) {
        registry.removeValue(forKey: windowId)
        if current.windowId == windowId {
            current = normal
            browser_set_focused_window(0)
        }
    }

    /// Put the normal window state back in the registry after an unexpected host
    /// window teardown (dock-click reopen).
    static func reregisterNormal() {
        registry[0] = normal
    }

    let windowId: Int32
    let isPrivate: Bool

    @Published var tabs: [TabItem] = []
    /// Tabs shown in little windows (P1-45). Never in a sidebar section; the
    /// little window's toolbar reads its tab from here.
    @Published var littleTabs: [TabItem] = []
    @Published var spaces: [SpaceItem] = []
    @Published var folders: [FolderItem] = []
    @Published var activeTabId: Int32?
    @Published var activeSpaceId: Int32 = 1
    /// Set when the core discards a tab's browser (ADR-0033); `RootView` detaches
    /// the CEF host in response.
    @Published var suspendedTabId: Int32?

    /// Sidebar pinned (always visible) vs. collapsed to hover-reveal. Persisted
    /// across launches, like the sidebar width (`RootView`); defaults to pinned.
    @Published var isSidebarPinned =
        UserDefaults.standard.object(forKey: BrowserState.sidebarPinnedKey) as? Bool ?? true
    /// Whether the collapsed sidebar is *hover-revealed*. Pointer-driven only
    /// (`revealSidebar` in, `RootView`'s dismissal monitor out). The empty-canvas
    /// auto-float is a separate, derived cause - see `isFloatingSidebarShown`.
    @Published var isSidebarRevealed = false

    private static let sidebarPinnedKey = BrowserProfile.defaultsKey("sidebarPinned")
    /// When enabled the address bar floats at the top of the page viewport
    /// (Safari-style) instead of living in the sidebar.
    @Published var isViewportURLBarEnabled =
        UserDefaults.standard.object(forKey: BrowserState.viewportURLBarKey) as? Bool ?? false
    /// When enabled, tint the sidebar to the active tab's sampled page color.
    @Published var isSidebarPageTintEnabled =
        UserDefaults.standard.object(forKey: BrowserState.sidebarPageTintKey) as? Bool ?? false

    private static let viewportURLBarKey = BrowserProfile.defaultsKey("viewportURLBar")
    private static let sidebarPageTintKey = BrowserProfile.defaultsKey("sidebarPageTint")

    init(windowId: Int32, isPrivate: Bool) {
        self.windowId = windowId
        self.isPrivate = isPrivate
    }

    /// Mirrors the window's fullscreen state (set by `AppDelegate`) so the
    /// custom traffic lights step aside - in fullscreen the OS auto-hiding
    /// titlebar shows the real buttons (see `TrafficLights`).
    @Published var isFullscreen = false

    /// HTML5 / browser content fullscreen (video element, Fullscreen API).
    /// Distinct from `isFullscreen` (OS window). The window controller enters
    /// real macOS fullscreen when this goes true; RootView hides chrome so
    /// the page fills the screen.
    @Published var isContentFullscreen = false

    /// Whether the History surface is shown over the content area (a native
    /// app-UI page, not a tab - ADR-0010). `RootView` reveals/hides its host on
    /// this; opening a tab or Escape closes it. It also counts as covering the
    /// canvas, so the empty-state floating sidebar tucks away while it's up and
    /// returns when it closes (derived - see `isFloatingSidebarShown`).
    @Published var isHistoryOpen = false

    /// Whether the Passwords (vault) surface is showing (ADR-0010/0016). Same
    /// content-area-surface model as History (and same empty-canvas rule).
    @Published var isVaultOpen = false

    /// Whether the Settings surface is showing (ADR-0010).
    @Published var isSettingsOpen = false

    // ─── Adblock (ADR-0020) ────────────────────────────────────────────────
    // Mirrors the core's adblock state for the sidebar shield + menu items. The
    // count and per-site state are for the *active* tab; refreshed off the event
    // stream (load start/end, tab switch) via `refreshAdblock()`.

    /// Global blocking switch (on by default).
    @Published var adblockEnabled = true
    /// Requests blocked on the active tab since its last navigation.
    @Published var adblockBlockedCount = 0
    /// Whether blocking is turned **off** for the active tab's site (allowlisted).
    @Published var adblockSiteOff = false
    /// Cookie-consent handling (reject + hide) switch (on by default; ADR-0026).
    @Published var consentEnabled = true
    /// Consent banners auto-rejected on the active tab since its last navigation.
    @Published var consentRejectedCount = 0

    // ─── Vault quick access (P2-34, ADR-0029) ──────────────────────────────
    // Backs the URL-bar key button + current-site logins popover. Refreshed off
    // the event stream (load start/end, tab switch) like the adblock shield, and
    // on any vault lock-state change (VaultModel.reload → refreshVaultMatches).

    /// Whether the vault has been set up (a button only appears once it is).
    @Published var vaultConfigured = false
    /// Whether the vault is locked (the popover offers unlock instead of a list).
    @Published var vaultLocked = true
    /// Logins matching the active tab's host (metadata only; empty when locked).
    @Published var vaultMatches: [VaultLogin] = []

    var activeTab: TabItem? {
        guard let activeTabId else { return nil }
        return tabs.first { $0.id == activeTabId }
    }

    var activeSpace: SpaceItem? { spaces.first { $0.id == activeSpaceId } }

    // Folders / tabs of the active space, split into the sidebar's sections.
    var activeFolders: [FolderItem] { folders.filter { $0.space == activeSpaceId } }
    var rootFolders: [FolderItem] { rootFolders(inSpace: activeSpaceId) }
    // Space-agnostic: a folder's parent uniquely determines its space, so filtering
    // by parent alone is correct for any space (the sidebar filmstrip renders every
    // space's tree at once, ADR-0043).
    func childFolders(of parent: Int32) -> [FolderItem] {
        folders.filter { $0.parent == parent }
    }

    // Per-space section accessors - the sidebar renders each space's tree in a
    // side-by-side filmstrip (ADR-0043), so it asks for a specific space rather
    // than the active one. The `active…` / bare computed forms above are the
    // `inSpace: activeSpaceId` case, kept for drag, selection, and the self-tests.
    func rootFolders(inSpace s: Int32) -> [FolderItem] {
        folders.filter { $0.space == s && $0.parent == nil }
    }
    func looseSavedTabs(inSpace s: Int32) -> [TabItem] {
        tabs.filter { $0.space == s && $0.pinned && !$0.favorite && $0.folder == nil }
    }
    func openTabs(inSpace s: Int32) -> [TabItem] {
        tabs.filter { $0.space == s && !$0.pinned && !$0.favorite }
    }
    /// Maximum visible favorites - 3×2 grid; matches core `MAX_VISIBLE_FAVORITES`.
    static let maxFavoriteTabs = 6

    /// Arc-style pinned icons - global across spaces (ADR-0032 / P2-5).
    var favoriteTabs: [TabItem] {
        tabs.filter { $0.favorite }
    }
    var canPinMoreFavorites: Bool { favoriteTabs.count < Self.maxFavoriteTabs }
    var looseSavedTabs: [TabItem] { looseSavedTabs(inSpace: activeSpaceId) }
    var openTabs: [TabItem] { openTabs(inSpace: activeSpaceId) }
    /// Whether any ephemeral tab exists in any space - saved tabs alone don't
    /// count as "open."
    var hasOpenTabs: Bool { tabs.contains { !$0.pinned && !$0.favorite } }

    func tabs(inFolder folder: Int32) -> [TabItem] { tabs.filter { $0.folder == folder } }

    /// The active tab, if it lives inside `folderId` or any of its nested
    /// descendant folders - the tab a collapsed folder peeks out so the current
    /// tab is never hidden (ADR-0039). `nil` while a surface covers the canvas
    /// (no tab reads as active then) or when the active tab is elsewhere. Walks
    /// up the tab's folder-parent chain, so the *nearest* collapsed ancestor is
    /// the one that peeks it.
    func activeDescendantTab(of folderId: Int32) -> TabItem? {
        guard !isSurfaceOpen, let activeTabId,
            let tab = tabs.first(where: { $0.id == activeTabId }),
            var current = tab.folder
        else { return nil }
        while true {
            if current == folderId { return tab }
            guard let parent = folders.first(where: { $0.id == current })?.parent else {
                return nil
            }
            current = parent
        }
    }

    private func appendFolderTabOrder(_ folderId: Int32, into ids: inout [Int32]) {
        for child in childFolders(of: folderId) {
            appendFolderTabOrder(child.id, into: &ids)
        }
        ids.append(contentsOf: tabs(inFolder: folderId).map(\.id))
    }

    /// Sidebar row order (folders → loose saved → open) for shift-click range
    /// selection across sections.
    var sidebarTabOrder: [Int32] {
        var ids: [Int32] = []
        ids.append(contentsOf: favoriteTabs.map(\.id))
        for folder in rootFolders {
            appendFolderTabOrder(folder.id, into: &ids)
        }
        ids.append(contentsOf: looseSavedTabs.map(\.id))
        ids.append(contentsOf: openTabs.map(\.id))
        return ids
    }

    /// Cmd/shift multi-select for bulk folder/close/clear actions.
    @Published var selectedTabIds: Set<Int32> = []
    private var selectionAnchor: Int32?

    var hasMultiTabSelection: Bool { selectedTabIds.count > 1 }

    // ─── Event ingestion (from the FFI trampoline, on the main thread) ─────

    func handleEvent(type: Int32, id: Int32, payload: String?) {
        switch type {
        case 0: patch(id) { $0.title = payload ?? "" }  // TabTitleChanged
        case 1: patch(id) { $0.url = payload ?? "" }  // TabUrlChanged
        case 2:  // TabLoadStarted
            patch(id) {
                $0.isLoading = true
                $0.themeColor = ""
            }
            refreshAdblock()  // core reset the count at nav start → shows 0
            refreshVaultMatches()  // the host may have changed → re-match logins
        case 9: patch(id) { $0.themeColor = payload ?? "" }  // TabThemeColorChanged
        case 3:  // TabLoadEnded
            patch(id) { $0.isLoading = false }
            refreshAdblock()  // final blocked count for the page
            refreshVaultMatches()
        case 4:  // TabFaviconChanged
            FaviconLoader.shared.invalidate(host: payload ?? "")
            patch(id) { $0.faviconVersion += 1 }
        case 5: reloadStructure()  // StructureChanged
        case 6:  // TabNavStateChanged - payload = "bf" back/forward flags
            let flags = payload ?? "00"
            patch(id) {
                $0.canGoBack = flags.first == "1"
                $0.canGoForward = flags.dropFirst().first == "1"
            }
        case 7:  // OpenTabRequested - payload = url; id = 1 foreground / 0 background
            guard let url = payload else { return }
            (NSApp.delegate as? AppDelegate)?.openTab(url: url, windowId: windowId)
        case 8:  // TabSuspended - background discard (ADR-0033)
            suspendedTabId = id
        case 11:  // OpenLittleRequested - payload = url (a favorite's link left its site)
            guard let url = payload else { return }
            (NSApp.delegate as? AppDelegate)?.openLittleWindow(url: url)
        case 10:  // FullscreenChanged - id = 1 enter / 0 exit (HTML5 content)
            let want = id != 0
            guard isContentFullscreen != want else { return }
            isContentFullscreen = want
        default: break
        }
    }

    private func patch(_ id: Int32, _ mutate: (inout TabItem) -> Void) {
        if let index = tabs.firstIndex(where: { $0.id == id }) {
            mutate(&tabs[index])
            tabs = tabs
        } else if let index = littleTabs.firstIndex(where: { $0.id == id }) {
            mutate(&littleTabs[index])
        }
    }

    /// Rebuild the tree from the core's JSON snapshot, preserving per-tab UI state
    /// (favicon version) so already-loaded icons don't flicker.
    func reloadStructure() {
        if isPrivate {
            reloadPrivateStructure()
            return
        }
        guard let raw = browser_structure_json() else { return }
        defer { browser_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        guard let snap = try? Self.decoder.decode(Snapshot.self, from: data) else { return }

        let priorFavicon = Dictionary(
            (tabs + littleTabs).map { ($0.id, $0.faviconVersion) }
        ) { a, _ in a }
        let item = { (dto: Snapshot.TabDTO) in
            TabItem(
                id: dto.id,
                space: dto.space,
                folder: dto.folder,
                pinned: dto.pinned,
                favorite: dto.favorite,
                savedURL: dto.savedUrl,
                title: dto.title,
                url: dto.url,
                isLoading: dto.loading,
                canGoBack: dto.canGoBack,
                canGoForward: dto.canGoForward,
                faviconVersion: priorFavicon[dto.id] ?? 0
            )
        }
        let newTabs = snap.tabs.map(item)
        littleTabs = snap.littleTabs.map(item)
        let newSpaces = snap.spaces.map {
            SpaceItem(
                id: $0.id, name: $0.name, icon: $0.icon,
                pinnedExpanded: $0.pinnedExpanded)
        }
        let newFolders = snap.folders.map {
            FolderItem(
                id: $0.id, space: $0.space, name: $0.name, parent: $0.parent,
                expanded: $0.expanded)
        }

        // Spring the sidebar through structural changes (reorder / pin / create /
        // close / select), but not the first population at launch - rows shouldn't
        // fly in - and not under Reduce Motion (style §7). Only SwiftUI observers
        // (the sidebar list) animate; the CEF view swap is AppKit-driven and
        // unaffected. This ambient spring also drives the rows' enter/exit
        // transitions and the sliding active-tab highlight (`SidebarSelection`).
        let priorSpace = activeSpaceId
        let apply = {
            self.tabs = newTabs
            self.spaces = newSpaces
            self.folders = newFolders
            self.activeSpaceId = snap.activeSpace
            self.activeTabId = snap.activeTab
            if snap.activeSpace != priorSpace {
                self.clearTabSelection()
            } else {
                self.pruneTabSelection()
            }
        }
        if spaces.isEmpty || NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            apply()
        } else {
            withAnimation(.spring(response: 0.24, dampingFraction: 0.88), apply)
        }

        refreshAdblock()  // the active tab may have changed → re-read its shield state
        refreshVaultMatches()  // …and re-match logins for the new active host
    }

    // ─── Adblock ───────────────────────────────────────────────────────────

    /// Re-read the global switch and the active tab's blocked count + per-site
    /// state from the core. Cheap; driven off the event stream.
    func refreshAdblock() {
        adblockEnabled = browser_adblock_is_enabled()
        adblockBlockedCount = Int(browser_adblock_active_blocked_count())
        if isPrivate {
            adblockSiteOff = browser_private_adblock_site_allowlisted(windowId)
        } else {
            adblockSiteOff = browser_adblock_active_site_allowlisted()
        }
        consentEnabled = browser_consent_is_enabled()
        consentRejectedCount = Int(browser_consent_active_rejected_count())
    }

    private func reloadPrivateStructure() {
        guard let raw = browser_private_structure_json(windowId) else { return }
        defer { browser_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        guard let snap = try? Self.privateDecoder.decode(PrivateSnapshot.self, from: data) else {
            return
        }

        let priorFavicon = Dictionary(tabs.map { ($0.id, $0.faviconVersion) }) { a, _ in a }
        tabs = snap.tabs.map { dto in
            TabItem(
                id: dto.id,
                space: 0,
                folder: nil,
                pinned: false,
                favorite: false,
                savedURL: nil,
                title: dto.title,
                url: dto.url,
                isLoading: dto.loading,
                canGoBack: dto.canGoBack,
                canGoForward: dto.canGoForward,
                faviconVersion: priorFavicon[dto.id] ?? 0
            )
        }
        activeTabId = snap.activeTab
        spaces = []
        folders = []
        activeSpaceId = 0
        refreshAdblock()
        refreshVaultMatches()
    }

    // ─── Vault quick access ──────────────────────────────────────────────────

    /// Re-read vault lock/configured state and the active tab's matching logins.
    /// Cheap (an in-memory lookup); driven off the event stream next to
    /// `refreshAdblock`, and on any lock-state change from the Passwords page.
    func refreshVaultMatches() {
        vaultConfigured = browser_vault_is_configured()
        vaultLocked = browser_vault_is_locked()
        let host = activeTab?.host ?? ""
        let vaultAllowed = !isPrivate || browser_settings_private_vault_enabled()
        vaultMatches =
            (vaultLocked || host.isEmpty || !vaultAllowed)
            ? [] : Self.fetchMatches(host: host)
    }

    private static func fetchMatches(host: String) -> [VaultLogin] {
        guard let raw = host.withCString({ browser_vault_lookup($0) }) else { return [] }
        defer { browser_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        return (try? JSONDecoder().decode([VaultLogin].self, from: data)) ?? []
    }

    /// Fill the active tab's login form from vault login `id` - the popover's Fill
    /// action. The secret crosses in the core on this gesture (ADR-0029); nothing
    /// comes back. The core records the use itself. Main thread only.
    func fillActiveTab(id: String) {
        id.withCString { browser_vault_fill_active_tab($0) }
        refreshVaultMatches()
    }

    /// Record that login `id` was just used, for the gestures that live entirely
    /// in the host - Copy username / Copy password (ADR-0048). Re-reads the
    /// matches so the popover reflects the new order without waiting for a
    /// navigation. Main thread only.
    func noteVaultUse(id: String) {
        id.withCString { browser_vault_note_used($0) }
        refreshVaultMatches()
    }

    /// Toggle cookie-consent handling (menu "Block Cookie Notices"). The core
    /// reloads the active tab so reject + hide apply on screen (ADR-0026).
    func toggleCookieConsent() {
        browser_consent_set_enabled(!consentEnabled)
        consentEnabled = browser_consent_is_enabled()
    }

    /// Toggle blocking globally (menu "Block Ads and Trackers"). The core reloads
    /// the active tab; the resulting load events refresh the count.
    func toggleAdblockGlobal() {
        browser_adblock_set_enabled(!adblockEnabled)
        refreshAdblock()
    }

    /// Turn blocking off/on for the active tab's site (the shield toggle / menu
    /// "Block Ads on This Site"). The core reloads the tab so it applies on screen.
    func toggleAdblockForSite() {
        if isPrivate {
            browser_private_adblock_set_site_allowlisted(windowId, !adblockSiteOff)
        } else {
            browser_adblock_set_active_site_allowlisted(!adblockSiteOff)
        }
        refreshAdblock()
    }

    private static let privateDecoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    // ─── Sidebar reveal ────────────────────────────────────────────────────

    func toggleSidebarPinned() {
        // Pinning/unpinning moves which sidebar instance is on-screen; end an
        // in-progress inline URL edit so it can't be left focused off-screen.
        URLBarModel.shared.endEditing()
        isSidebarPinned.toggle()
        UserDefaults.standard.set(isSidebarPinned, forKey: Self.sidebarPinnedKey)
        isSidebarRevealed = false
    }

    func setViewportURLBarEnabled(_ enabled: Bool) {
        guard enabled != isViewportURLBarEnabled else { return }
        URLBarModel.shared.endEditing()
        isViewportURLBarEnabled = enabled
        UserDefaults.standard.set(enabled, forKey: Self.viewportURLBarKey)
    }

    func toggleViewportURLBar() {
        setViewportURLBarEnabled(!isViewportURLBarEnabled)
    }

    func setSidebarPageTintEnabled(_ enabled: Bool) {
        guard enabled != isSidebarPageTintEnabled else { return }
        isSidebarPageTintEnabled = enabled
        UserDefaults.standard.set(enabled, forKey: Self.sidebarPageTintKey)
    }

    // ─── App-UI surfaces (History / Passwords) ─────────────────────────────
    // A native content-area surface and a tab are mutually exclusive: opening one
    // deselects the tab (visually - the core keeps its active tab, so dismissing
    // returns to it, ADR-0010), and the two surfaces are exclusive with each
    // other (only one covers the canvas). `selectTab` is the reverse entry point:
    // a sidebar tab click dismisses the surface and returns to the page.

    /// True while any app-UI surface covers the content area. The sidebar hides
    /// its active-tab highlight while this holds, so no tab reads as selected.
    var isSurfaceOpen: Bool { isHistoryOpen || isVaultOpen || isSettingsOpen }

    func toggleHistory() { isHistoryOpen ? closeHistory() : openHistory() }
    func toggleVault() { isVaultOpen ? closeVault() : openVault() }
    func toggleSettings() { isSettingsOpen ? closeSettings() : openSettings() }

    /// Show a surface, closing the other one first - only one covers the canvas.
    /// Goes straight to the booleans (not through `close*`) so switching between
    /// surfaces never bounces focus down to the web view in between.
    func openHistory() {
        URLBarModel.shared.endEditing()
        isVaultOpen = false
        isSettingsOpen = false
        isHistoryOpen = true
    }

    func openVault() {
        URLBarModel.shared.endEditing()
        isHistoryOpen = false
        isSettingsOpen = false
        isVaultOpen = true
    }

    func openSettings() {
        URLBarModel.shared.endEditing()
        isHistoryOpen = false
        isVaultOpen = false
        isSettingsOpen = true
    }

    func closeHistory() {
        guard isHistoryOpen else { return }
        isHistoryOpen = false
        restoreWebFocus()
    }

    func closeVault() {
        guard isVaultOpen else { return }
        isVaultOpen = false
        restoreWebFocus()
    }

    func closeSettings() {
        guard isSettingsOpen else { return }
        isSettingsOpen = false
        restoreWebFocus()
    }

    /// Dismiss whichever surface is showing - used by ⌘W and by paths that need
    /// the canvas back (opening a new tab, reopening a closed one).
    func closeSurfaces() {
        closeHistory()
        closeVault()
        closeSettings()
    }

    /// Handle a sidebar tab click with modifier keys for multi-select.
    func handleTabClick(_ id: Int32) {
        let flags = NSEvent.modifierFlags.intersection(.deviceIndependentFlagsMask)
        let ordered = sidebarTabOrder

        if flags.contains(.command) {
            if selectedTabIds.contains(id) {
                selectedTabIds.remove(id)
            } else {
                if selectedTabIds.isEmpty, let active = activeTabId {
                    selectedTabIds.insert(active)
                }
                selectedTabIds.insert(id)
            }
            selectionAnchor = id
            return
        }

        if flags.contains(.shift) {
            let anchor = selectionAnchor ?? activeTabId ?? id
            guard let from = ordered.firstIndex(of: anchor),
                let to = ordered.firstIndex(of: id)
            else {
                clearTabSelection()
                selectTab(id)
                selectionAnchor = id
                return
            }
            let lo = min(from, to)
            let hi = max(from, to)
            selectedTabIds = Set(ordered[lo...hi])
            selectionAnchor = anchor
            return
        }

        clearTabSelection()
        selectTab(id)
        selectionAnchor = id
    }

    func clearTabSelection() {
        selectedTabIds.removeAll()
        selectionAnchor = nil
    }

    private func pruneTabSelection() {
        let live = Set(tabs.map(\.id))
        selectedTabIds = selectedTabIds.filter { live.contains($0) }
        if selectedTabIds.isEmpty { selectionAnchor = nil }
    }

    /// Select a tab from the sidebar. A tab and a surface are mutually exclusive,
    /// so dismiss any open surface and hand keyboard focus back to the page. The
    /// core already refocuses when the *visible tab changes* (`tabs::set_visible`);
    /// the explicit focus call covers re-selecting the tab already active behind
    /// the surface, where the visible tab doesn't change. Main thread only.
    func selectTab(_ id: Int32) {
        let hadSurface = isSurfaceOpen
        isHistoryOpen = false
        isVaultOpen = false
        isSettingsOpen = false
        if isPrivate {
            browser_private_tab_select(windowId, id)
        } else {
            browser_tab_select(id)
        }
        if hadSurface { browser_focus_active_tab() }
    }

    /// Return keyboard focus to the active tab's web view once no surface covers
    /// it (no-op in the empty state). Surface-to-surface switches go through
    /// `open*`, which never calls this, so focus stays on the incoming surface.
    private func restoreWebFocus() {
        if !isSurfaceOpen { browser_focus_active_tab() }
    }

    /// Hover-reveal the collapsed sidebar. `RootView` owns the dismissal timing -
    /// it has the (resizable) sidebar geometry - and flips `isSidebarRevealed`
    /// back off once the pointer leaves the sidebar's hover zone for a grace
    /// period.
    func revealSidebar() {
        guard !isSidebarPinned, !isSidebarRevealed else { return }
        isSidebarRevealed = true
    }

    /// The canvas is empty: no active tab and nothing covering it. With nothing on
    /// the canvas the collapsed sidebar auto-floats (Arc) - the only way to reach a
    /// saved tab. A History/Vault panel counts as covering it, so it isn't empty.
    var isEmptyCanvas: Bool {
        activeTabId == nil && !isHistoryOpen && !isVaultOpen && !isSettingsOpen
    }

    /// Whether the floating (unpinned) sidebar is on-screen. The single rule:
    /// unpinned, and either hover-revealed or floating over an empty canvas. A pure
    /// function of state - no stored copy to keep in sync, no override. `RootView`
    /// animates the floating host to match this. Suppressed during content
    /// fullscreen so video fills the whole screen.
    var isFloatingSidebarShown: Bool {
        !isContentFullscreen && !isSidebarPinned && (isSidebarRevealed || isEmptyCanvas)
    }
}

/// The core's structure snapshot (ADR-0005). Keys are snake_case on the wire;
/// decoded with `.convertFromSnakeCase`.
private struct Snapshot: Decodable {
    let activeSpace: Int32
    let activeTab: Int32?
    let spaces: [SpaceDTO]
    let folders: [FolderDTO]
    let tabs: [TabDTO]
    let littleTabs: [TabDTO]

    struct SpaceDTO: Decodable {
        let id: Int32
        let name: String
        let icon: String
        let pinnedExpanded: Bool
    }
    struct FolderDTO: Decodable {
        let id: Int32
        let space: Int32
        let name: String
        let parent: Int32?
        let expanded: Bool
    }
    struct TabDTO: Decodable {
        let id: Int32
        let space: Int32
        let folder: Int32?
        let pinned: Bool
        let favorite: Bool
        let savedUrl: String?
        let url: String
        let title: String
        let loading: Bool
        let canGoBack: Bool
        let canGoForward: Bool
    }
}

private struct PrivateSnapshot: Decodable {
    let windowId: Int32
    let activeTab: Int32?
    let tabs: [PrivateTabDTO]

    struct PrivateTabDTO: Decodable {
        let id: Int32
        let url: String
        let title: String
        let loading: Bool
        let canGoBack: Bool
        let canGoForward: Bool
    }
}

/// C-ABI trampoline for the core's event stream (ADR-0003). Registered once at
/// launch via `browser_set_event_callback`. A non-capturing global, so it
/// coerces to the expected `@convention(c)` pointer. Invoked on the main thread.
func browserTabEventCallback(
    _ type: Int32,
    _ windowId: Int32,
    _ id: Int32,
    _ payload: UnsafePointer<CChar>?
) {
    let text = payload.map { String(cString: $0) }
    MainActor.assumeIsolated {
        BrowserState.registry[windowId]?.handleEvent(type: type, id: id, payload: text)
    }
}

/// Turns command-bar input into a navigation. `route` selects the destination:
/// ⌘T / empty state opens a **new tab**; ⌘L / the URL chip edits the **current
/// tab** in place. URLs load directly; anything else becomes a web search.
enum Navigation {
    /// Where a command-bar commit lands. The destination is part of the core
    /// overlay state (ADR-0068), never a global the surfaces write.
    enum Route { case newTab, currentTab, littleWindow }

    /// Commit typed input or a suggestion's URL from the surface that is up.
    static func open(_ input: String) {
        let query = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return }
        let url = resolve(query)
        MainActor.assumeIsolated { OverlayMachine.commit(url) }
    }

    private static func resolve(_ query: String) -> String {
        if query.hasPrefix("http://") || query.hasPrefix("https://") {
            return query
        }
        if !query.contains(" "), query.contains(".") {
            return "https://" + query
        }
        return SearchNavigation.url(for: query)
    }
}
