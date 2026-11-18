import AppKit

/// The host side of the core UI surface state machine (P1-46, ADR-0068).
/// Every intent (open the command bar, edit the address, open a page, show
/// find, close one, commit) goes through `send`; the core returns the whole
/// surface state and effects, and the host renders that state. No surface sets
/// another surface's state, and nothing else decides where a commit lands.
@MainActor
enum OverlayMachine {
    /// The input surface that is up (app-wide, at most one).
    enum State: Equatable {
        case none
        case commandBar(Navigation.Route)
        case addressEdit
    }

    /// An app page covering a window's web view.
    enum Page: String, Decodable {
        case history, passwords, settings
    }

    struct Snapshot: Equatable {
        var overlay: State = .none
        /// The window whose find bar is up.
        var find: Int32?
        /// Windows showing a page; absent means the web view.
        var pages: [Int32: Page] = [:]
    }

    private(set) static var current = Snapshot()
    static var state: State { current.overlay }

    /// Renders a transition. Installed by `AppDelegate`; `initialQuery`
    /// prefills an opening command bar.
    static var render: ((_ from: Snapshot, _ to: Snapshot, _ initialQuery: String) -> Void)?

    /// The window id little windows use here (normal is 0, private >= 1).
    static func littleWindowId(tab: Int32) -> Int32 { -(tab + 1) }

    static func openCommandBar(_ dest: Navigation.Route, initialQuery: String = "") {
        send(0, arg: code(dest), initialQuery: initialQuery)
    }
    static func beginAddressEdit(window: Int32) { send(1, window: window) }
    static func closeCommandBar() { send(2, arg: 0) }
    static func closeAddressEdit() { send(2, arg: 1) }
    /// Go to `url` from whichever input surface is up. Dropped when none is.
    static func commit(_ url: String) { send(3, url: url) }
    static func openPage(_ page: Page, window: Int32) { send(4, arg: code(page), window: window) }
    static func closePage(_ page: Page, window: Int32) { send(5, arg: code(page), window: window) }
    static func showWeb(window: Int32) { send(6, window: window) }
    static func openFind(window: Int32) { send(7, window: window) }
    static func closeFind() { send(8) }
    static func windowClosed(_ window: Int32) { send(9, window: window) }

    private static func send(
        _ input: Int32, arg: Int32 = 0, window: Int32 = 0, url: String = "",
        initialQuery: String = ""
    ) {
        guard let raw = url.withCString({ browser_surface_send(input, arg, window, $0) }) else {
            return
        }
        let data = Data(String(cString: raw).utf8)
        browser_string_free(raw)
        guard let reply = try? JSONDecoder().decode(Reply.self, from: data) else { return }
        let old = current
        current = reply.ui.snapshot
        if old != current { render?(old, current, initialQuery) }
        for effect in reply.effects { run(effect) }
    }

    private static func run(_ effect: Reply.Effect) {
        let app = NSApp.delegate as? AppDelegate
        switch effect.dest {
        case "current_tab": effect.url.withCString { browser_navigate($0) }
        case "little_window": app?.openLittleWindow(url: effect.url)
        default: app?.openTab(url: effect.url, windowId: BrowserState.current.windowId)
        }
    }

    private static func code(_ dest: Navigation.Route) -> Int32 {
        switch dest {
        case .newTab: 0
        case .currentTab: 1
        case .littleWindow: 2
        }
    }

    private static func code(_ page: Page) -> Int32 {
        switch page {
        case .history: 0
        case .passwords: 1
        case .settings: 2
        }
    }

    private struct Reply: Decodable {
        struct Overlay: Decodable {
            let kind: String
            let dest: String?

            var state: State {
                switch kind {
                case "command_bar":
                    switch dest {
                    case "current_tab": .commandBar(.currentTab)
                    case "little_window": .commandBar(.littleWindow)
                    default: .commandBar(.newTab)
                    }
                case "address_edit": .addressEdit
                default: .none
                }
            }
        }
        struct Ui: Decodable {
            let overlay: Overlay
            let find: Int32?
            let pages: [String: Page]

            var snapshot: Snapshot {
                Snapshot(
                    overlay: overlay.state, find: find,
                    pages: Dictionary(
                        uniqueKeysWithValues: pages.compactMap { key, page in
                            Int32(key).map { ($0, page) }
                        }))
            }
        }
        struct Effect: Decodable {
            let kind: String
            let dest: String
            let url: String
        }
        let ui: Ui
        let effects: [Effect]
    }
}
