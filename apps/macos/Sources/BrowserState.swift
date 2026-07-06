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
    @Published var isSidebarRevealed = false

    private static let sidebarPinnedKey = "sidebarPinned"

    /// Mirrors the window's fullscreen state (set by `AppDelegate`) so the
    /// custom traffic lights step aside — in fullscreen the OS auto-hiding
    /// titlebar shows the real buttons (see `TrafficLights`).
    @Published var isFullscreen = false

    /// Whether the History surface is shown over the content area (a native
    /// app-UI page, not a tab — ADR-0010). `RootView` reveals/hides its host on
    /// this; opening a tab or Escape closes it.
    @Published var isHistoryOpen = false

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
        case 2: patch(id) { $0.isLoading = true } // TabLoadStarted
        case 3: patch(id) { $0.isLoading = false } // TabLoadEnded
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
        // close), but not the first population at launch — rows shouldn't fly in.
        // Only SwiftUI observers (the sidebar list) animate; the CEF view swap is
        // AppKit-driven and unaffected.
        let apply = {
            self.tabs = newTabs
            self.spaces = newSpaces
            self.folders = newFolders
            self.activeSpaceId = snap.activeSpace
            self.activeTabId = snap.activeTab
        }
        if spaces.isEmpty {
            apply()
        } else {
            withAnimation(.spring(response: 0.32, dampingFraction: 0.86), apply)
        }
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

    // ─── History surface ───────────────────────────────────────────────────

    func toggleHistory() { isHistoryOpen.toggle() }

    func closeHistory() {
        if isHistoryOpen { isHistoryOpen = false }
    }

    /// Hover-reveal the collapsed sidebar. `RootView` owns the dismissal timing —
    /// it has the (resizable) sidebar geometry — and flips `isSidebarRevealed`
    /// back off once the pointer leaves the sidebar's hover zone for a grace
    /// period.
    func revealSidebar() {
        guard !isSidebarPinned, !isSidebarRevealed else { return }
        isSidebarRevealed = true
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
