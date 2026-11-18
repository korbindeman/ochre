import AppKit

/// The host side of the core overlay state machine (P1-46, ADR-0068). Every
/// intent (open the command bar, edit the address, close one, commit) goes
/// through `send`; the core returns the new state and effects, and the host
/// renders that state. No surface sets another surface's state, and nothing
/// else decides where a commit lands.
@MainActor
enum OverlayMachine {
    enum State: Equatable {
        case none
        case commandBar(Navigation.Route)
        case addressEdit
    }

    private(set) static var state: State = .none

    /// Renders a transition. Installed by `AppDelegate`, which owns the
    /// command bar; `initialQuery` prefills an opening command bar.
    static var render: ((_ from: State, _ to: State, _ initialQuery: String) -> Void)?

    static func openCommandBar(_ dest: Navigation.Route, initialQuery: String = "") {
        send(input: 0, arg: code(dest), initialQuery: initialQuery)
    }

    static func beginAddressEdit() { send(input: 1, arg: 0) }
    static func closeCommandBar() { send(input: 2, arg: 0) }
    static func closeAddressEdit() { send(input: 2, arg: 1) }

    /// Go to `url` from whichever surface is up. Dropped when none is.
    static func commit(_ url: String) { send(input: 3, arg: 0, url: url) }

    private static func send(input: Int32, arg: Int32, url: String = "", initialQuery: String = "") {
        guard let raw = url.withCString({ browser_overlay_send(input, arg, $0) }) else { return }
        let data = Data(String(cString: raw).utf8)
        browser_string_free(raw)
        guard let reply = try? JSONDecoder().decode(Reply.self, from: data) else { return }
        let old = state
        state = reply.overlay.state
        if old != state { render?(old, state, initialQuery) }
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
        struct Effect: Decodable {
            let kind: String
            let dest: String
            let url: String
        }
        let overlay: Overlay
        let effects: [Effect]
    }
}
