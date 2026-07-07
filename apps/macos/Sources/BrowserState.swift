import AppKit
import SwiftUI

/// One tab, mirrored from the core's tree over the event stream (ADR-0003/0005).
struct TabItem: Identifiable, Equatable {
    let id: Int32
    var space: Int32
    /// The folder this tab lives in (pinned tabs only), or nil for a loose tab.
    var folder: Int32?
    /// Persistent (pinned) vs. ephemeral ("today"). Only pinned tabs join folders.
    var pinned: Bool
    var title: String = ""
    var url: String = ""
    var isLoading: Bool = false
    /// History enablement, streamed from the core, driving the sidebar's
    /// back/forward controls (disabled when there's nowhere to go).
    var canGoBack: Bool = false
    var canGoForward: Bool = false
    /// Bumped when the engine captures a fresh favicon, to retrigger the load.
    var faviconVersion: Int = 0

    var host: String { URLComponents(string: url)?.host ?? "" }

    var displayTitle: String {
        if !title.isEmpty { return title }
        if !host.isEmpty { return host }
        return "New Tab"
    }
}

/// A space — a switchable set of tabs (with its own pinned + today sections).
/// `icon` is the emoji shown in the space switcher.
struct SpaceItem: Identifiable, Equatable {
    let id: Int32
    var name: String
    var icon: String
}

/// A folder inside a space's pinned section; holds pinned tabs.
struct FolderItem: Identifiable, Equatable {
    let id: Int32
    var space: Int32
    var name: String
}

/// Observable UI state, a projection of the core-owned tree. Structure (spaces /
/// folders / tab membership / active) is rebuilt wholesale from a JSON snapshot
/// on `StructureChanged`; hot per-tab content is patched in place (ADR-0005).
@MainActor
final class BrowserState: ObservableObject {
    static let shared = BrowserState()

    @Published var tabs: [TabItem] = []
    @Published var spaces: [SpaceItem] = []
    @Published var folders: [FolderItem] = []
    @Published var activeTabId: Int32?
    @Published var activeSpaceId: Int32 = 1

    /// Sidebar pinned (always visible) vs. collapsed to hover-reveal. Persisted
    /// across launches, like the sidebar width (`RootView`); defaults to pinned.
    @Published var isSidebarPinned =
        UserDefaults.standard.object(forKey: BrowserState.sidebarPinnedKey) as? Bool ?? true
    /// Whether the collapsed sidebar is *hover-revealed*. Pointer-driven only
    /// (`revealSidebar` in, `RootView`'s dismissal monitor out). The empty-canvas
    /// auto-float is a separate, derived cause — see `isFloatingSidebarShown`.
    @Published var isSidebarRevealed = false

    private static let sidebarPinnedKey = BrowserProfile.defaultsKey("sidebarPinned")

    /// Mirrors the window's fullscreen state (set by `AppDelegate`) so the
    /// custom traffic lights step aside — in fullscreen the OS auto-hiding
    /// titlebar shows the real buttons (see `TrafficLights`).
    @Published var isFullscreen = false

    /// Whether the History surface is shown over the content area (a native
    /// app-UI page, not a tab — ADR-0010). `RootView` reveals/hides its host on
    /// this; opening a tab or Escape closes it. It also counts as covering the
    /// canvas, so the empty-state floating sidebar tucks away while it's up and
    /// returns when it closes (derived — see `isFloatingSidebarShown`).
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

    // Folders / tabs of the active space, split into the sidebar's sections.
    var activeFolders: [FolderItem] { folders.filter { $0.space == activeSpaceId } }
    var loosePinnedTabs: [TabItem] {
        tabs.filter { $0.space == activeSpaceId && $0.pinned && $0.folder == nil }
    }
    var todayTabs: [TabItem] { tabs.filter { $0.space == activeSpaceId && !$0.pinned } }
    func tabs(inFolder folder: Int32) -> [TabItem] { tabs.filter { $0.folder == folder } }

    // ─── Event ingestion (from the FFI trampoline, on the main thread) ─────

    func handleEvent(type: Int32, id: Int32, payload: String?) {
        switch type {
        case 0: patch(id) { $0.title = payload ?? "" } // TabTitleChanged
        case 1: patch(id) { $0.url = payload ?? "" } // TabUrlChanged
        case 2: // TabLoadStarted
            patch(id) { $0.isLoading = true }
            refreshAdblock() // core reset the count at nav start → shows 0
            refreshVaultMatches() // the host may have changed → re-match logins
        case 3: // TabLoadEnded
            patch(id) { $0.isLoading = false }
            refreshAdblock() // final blocked count for the page
            refreshVaultMatches()
        case 4: // TabFaviconChanged
            FaviconLoader.shared.invalidate(host: payload ?? "")
            patch(id) { $0.faviconVersion += 1 }
        case 5: reloadStructure() // StructureChanged
        case 6: // TabNavStateChanged — payload = "bf" back/forward flags
            let flags = payload ?? "00"
            patch(id) {
                $0.canGoBack = flags.first == "1"
                $0.canGoForward = flags.dropFirst().first == "1"
            }
        case 7: // OpenTabRequested — payload = url; id = 1 foreground / 0 background
            guard let url = payload else { return }
            // Background-tab opening isn't wired yet; foreground matches target="_blank".
            (NSApp.delegate as? AppDelegate)?.openTab(url: url)
        default: break
        }
    }

    private func patch(_ id: Int32, _ mutate: (inout TabItem) -> Void) {
        guard let index = tabs.firstIndex(where: { $0.id == id }) else { return }
        mutate(&tabs[index])
    }

    /// Rebuild the tree from the core's JSON snapshot, preserving per-tab UI state
    /// (favicon version) so already-loaded icons don't flicker.
    func reloadStructure() {
        guard let raw = browser_structure_json() else { return }
        defer { browser_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        guard let snap = try? Self.decoder.decode(Snapshot.self, from: data) else { return }

        let priorFavicon = Dictionary(tabs.map { ($0.id, $0.faviconVersion) }) { a, _ in a }
        let newTabs = snap.tabs.map { dto in
            TabItem(
                id: dto.id,
                space: dto.space,
                folder: dto.folder,
                pinned: dto.pinned,
                title: dto.title,
                url: dto.url,
                isLoading: dto.loading,
                canGoBack: dto.canGoBack,
                canGoForward: dto.canGoForward,
                faviconVersion: priorFavicon[dto.id] ?? 0
            )
        }
        let newSpaces = snap.spaces.map { SpaceItem(id: $0.id, name: $0.name, icon: $0.icon) }
        let newFolders = snap.folders.map { FolderItem(id: $0.id, space: $0.space, name: $0.name) }

        // Spring the sidebar through structural changes (reorder / pin / create /
        // close / select), but not the first population at launch — rows shouldn't
        // fly in — and not under Reduce Motion (style §7). Only SwiftUI observers
        // (the sidebar list) animate; the CEF view swap is AppKit-driven and
        // unaffected. This ambient spring also drives the rows' enter/exit
        // transitions and the sliding active-tab highlight (`SidebarSelection`).
        let apply = {
            self.tabs = newTabs
            self.spaces = newSpaces
            self.folders = newFolders
            self.activeSpaceId = snap.activeSpace
            self.activeTabId = snap.activeTab
        }
        if spaces.isEmpty || NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            apply()
        } else {
            withAnimation(.spring(response: 0.24, dampingFraction: 0.88), apply)
        }

        refreshAdblock() // the active tab may have changed → re-read its shield state
        refreshVaultMatches() // …and re-match logins for the new active host
    }

    // ─── Adblock ───────────────────────────────────────────────────────────

    /// Re-read the global switch and the active tab's blocked count + per-site
    /// state from the core. Cheap; driven off the event stream.
    func refreshAdblock() {
        adblockEnabled = browser_adblock_is_enabled()
        adblockBlockedCount = Int(browser_adblock_active_blocked_count())
        adblockSiteOff = browser_adblock_active_site_allowlisted()
        consentEnabled = browser_consent_is_enabled()
    }

    // ─── Vault quick access ──────────────────────────────────────────────────

    /// Re-read vault lock/configured state and the active tab's matching logins.
    /// Cheap (an in-memory lookup); driven off the event stream next to
    /// `refreshAdblock`, and on any lock-state change from the Passwords page.
    func refreshVaultMatches() {
        vaultConfigured = browser_vault_is_configured()
        vaultLocked = browser_vault_is_locked()
        let host = activeTab?.host ?? ""
        vaultMatches = (vaultLocked || host.isEmpty) ? [] : Self.fetchMatches(host: host)
    }

    private static func fetchMatches(host: String) -> [VaultLogin] {
        guard let raw = host.withCString({ browser_vault_lookup($0) }) else { return [] }
        defer { browser_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        return (try? JSONDecoder().decode([VaultLogin].self, from: data)) ?? []
    }

    /// Fill the active tab's login form from vault login `id` — the popover's Fill
    /// action. The secret crosses in the core on this gesture (ADR-0029); nothing
    /// comes back. Main thread only.
    func fillActiveTab(id: String) {
        id.withCString { browser_vault_fill_active_tab($0) }
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
        browser_adblock_set_active_site_allowlisted(!adblockSiteOff)
        refreshAdblock()
    }

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

    // ─── App-UI surfaces (History / Passwords) ─────────────────────────────
    // A native content-area surface and a tab are mutually exclusive: opening one
    // deselects the tab (visually — the core keeps its active tab, so dismissing
    // returns to it, ADR-0010), and the two surfaces are exclusive with each
    // other (only one covers the canvas). `selectTab` is the reverse entry point:
    // a sidebar tab click dismisses the surface and returns to the page.

    /// True while any app-UI surface covers the content area. The sidebar hides
    /// its active-tab highlight while this holds, so no tab reads as selected.
    var isSurfaceOpen: Bool { isHistoryOpen || isVaultOpen || isSettingsOpen }

    func toggleHistory() { isHistoryOpen ? closeHistory() : openHistory() }
    func toggleVault() { isVaultOpen ? closeVault() : openVault() }
    func toggleSettings() { isSettingsOpen ? closeSettings() : openSettings() }

    /// Show a surface, closing the other one first — only one covers the canvas.
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

    /// Dismiss whichever surface is showing — used by ⌘W and by paths that need
    /// the canvas back (opening a new tab, reopening a closed one).
    func closeSurfaces() {
        closeHistory()
        closeVault()
        closeSettings()
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
        browser_tab_select(id)
        if hadSurface { browser_focus_active_tab() }
    }

    /// Return keyboard focus to the active tab's web view once no surface covers
    /// it (no-op in the empty state). Surface-to-surface switches go through
    /// `open*`, which never calls this, so focus stays on the incoming surface.
    private func restoreWebFocus() {
        if !isSurfaceOpen { browser_focus_active_tab() }
    }

    /// Hover-reveal the collapsed sidebar. `RootView` owns the dismissal timing —
    /// it has the (resizable) sidebar geometry — and flips `isSidebarRevealed`
    /// back off once the pointer leaves the sidebar's hover zone for a grace
    /// period.
    func revealSidebar() {
        guard !isSidebarPinned, !isSidebarRevealed else { return }
        isSidebarRevealed = true
    }

    /// The canvas is empty: no active tab and nothing covering it. With nothing on
    /// the canvas the collapsed sidebar auto-floats (Arc) — the only way to reach a
    /// saved tab. A History/Vault panel counts as covering it, so it isn't empty.
    var isEmptyCanvas: Bool {
        activeTabId == nil && !isHistoryOpen && !isVaultOpen && !isSettingsOpen
    }

    /// Whether the floating (unpinned) sidebar is on-screen. The single rule:
    /// unpinned, and either hover-revealed or floating over an empty canvas. A pure
    /// function of state — no stored copy to keep in sync, no override. `RootView`
    /// animates the floating host to match this.
    var isFloatingSidebarShown: Bool {
        !isSidebarPinned && (isSidebarRevealed || isEmptyCanvas)
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

    struct SpaceDTO: Decodable {
        let id: Int32
        let name: String
        let icon: String
    }
    struct FolderDTO: Decodable {
        let id: Int32
        let space: Int32
        let name: String
    }
    struct TabDTO: Decodable {
        let id: Int32
        let space: Int32
        let folder: Int32?
        let pinned: Bool
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
func browserTabEventCallback(_ type: Int32, _ id: Int32, _ payload: UnsafePointer<CChar>?) {
    let text = payload.map { String(cString: $0) }
    MainActor.assumeIsolated {
        BrowserState.shared.handleEvent(type: type, id: id, payload: text)
    }
}

/// Turns command-bar input into a navigation. `route` selects the destination:
/// ⌘T / empty state opens a **new tab**; ⌘L / the URL chip edits the **current
/// tab** in place. URLs load directly; anything else becomes a web search.
enum Navigation {
    enum Route { case newTab, currentTab }

    /// Where the next commit lands — set by the command bar before it opens.
    static var route: Route = .newTab

    static func open(_ input: String) {
        let query = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return }
        let url = resolve(query)
        MainActor.assumeIsolated {
            switch route {
            case .newTab:
                (NSApp.delegate as? AppDelegate)?.openTab(url: url)
            case .currentTab:
                url.withCString { browser_navigate($0) }
            }
        }
    }

    private static func resolve(_ query: String) -> String {
        if query.hasPrefix("http://") || query.hasPrefix("https://") {
            return query
        }
        if !query.contains(" "), query.contains(".") {
            return "https://" + query
        }
        let encoded = query.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) ?? query
        return "https://www.google.com/search?q=\(encoded)"
    }
}
