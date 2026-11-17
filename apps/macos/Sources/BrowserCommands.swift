import AppKit
import Foundation

/// A Sendable snapshot of one enabled browser command in the native menu.
/// The menu remains the canonical registry for title, availability, shortcut,
/// and dispatch; Cmd-T projects it instead of duplicating those rules.
private struct BrowserCommandDescriptor: Sendable {
    let id: String
    let title: String
    let path: String
    let searchText: String
}

/// Browser actions backed by AppDelegate-targeted native menu items. Because
/// discovery is recursive and validation happens for every query, commands
/// added to the menu automatically become searchable in Cmd-T when applicable.
struct BrowserCommandsProvider: SuggestionProvider {
    func suggestions(for query: String) async -> [Suggestion] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return [] }
        let commands = await MainActor.run { BrowserCommandMenu.snapshot() }

        return commands.compactMap { command in
            guard let match = FuzzyMatch.match(query: query, text: command.searchText) else {
                return nil
            }
            let titleMatches = FuzzyMatch.match(query: query, text: command.title)?.indices ?? []
            return Suggestion(
                id: "command:\(command.id)",
                icon: .system("command"),
                title: command.title,
                subtitle: command.path,
                run: { BrowserCommandMenu.run(command.id) },
                score: SuggestionScore.command + match.score,
                titleMatches: titleMatches
            )
        }
    }
}

/// Space names are dynamic state, not static menu items, so they join Cmd-T as
/// their own command source.
struct SpaceCommandsProvider: SuggestionProvider {
    func suggestions(for query: String) async -> [Suggestion] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return [] }
        let spaces = await MainActor.run { BrowserState.shared.spaces }

        return spaces.compactMap { space in
            let title = "Switch to \(space.name)"
            let searchable = "\(title) space \(space.icon)"
            guard let match = FuzzyMatch.match(query: query, text: searchable) else { return nil }
            let titleMatches = FuzzyMatch.match(query: query, text: title)?.indices ?? []
            return Suggestion(
                id: "space:\(space.id)",
                icon: .system("square.grid.2x2"),
                title: title,
                subtitle: "Space",
                run: { browser_select_space(space.id) },
                score: SuggestionScore.command + match.score,
                titleMatches: titleMatches
            )
        }
    }
}

@MainActor
private enum BrowserCommandMenu {
    /// Opening or quitting the shell from inside Cmd-T is either recursive or
    /// dangerously easy to trigger. All other enabled browser menu actions are
    /// valid commands, including future actions added after this provider.
    private static let excludedActions: Set<String> = [
        NSStringFromSelector(#selector(AppDelegate.toggleCommandBar)),
        NSStringFromSelector(#selector(AppDelegate.closeWindow(_:))),
        NSStringFromSelector(#selector(AppDelegate.quit(_:))),
    ]

    static func snapshot() -> [BrowserCommandDescriptor] {
        guard let menu = NSApp.mainMenu, let app = NSApp.delegate as? AppDelegate else { return [] }
        return collect(menu: menu, path: [], app: app)
    }

    static func run(_ id: String) {
        guard let menu = NSApp.mainMenu, let app = NSApp.delegate as? AppDelegate,
            let item = find(id: id, menu: menu, path: [], app: app),
            let action = item.action
        else { return }
        NSApp.sendAction(action, to: item.target, from: item)
    }

    private static func collect(
        menu: NSMenu, path: [String], app: AppDelegate
    ) -> [BrowserCommandDescriptor] {
        menu.items.flatMap { item -> [BrowserCommandDescriptor] in
            if let submenu = item.submenu {
                return collect(menu: submenu, path: path + [item.title], app: app)
            }
            guard isCommand(item, app: app), let action = item.action else { return [] }
            let menuPath = path.joined(separator: " › ")
            return [BrowserCommandDescriptor(
                id: commandId(action: action, tag: item.tag, path: menuPath),
                title: item.title,
                path: menuPath,
                searchText: "\(item.title) \(menuPath)"
            )]
        }
    }

    private static func find(
        id: String, menu: NSMenu, path: [String], app: AppDelegate
    ) -> NSMenuItem? {
        for item in menu.items {
            if let submenu = item.submenu,
                let found = find(id: id, menu: submenu, path: path + [item.title], app: app)
            {
                return found
            }
            guard let action = item.action else { continue }
            let menuPath = path.joined(separator: " › ")
            if commandId(action: action, tag: item.tag, path: menuPath) == id,
                isCommand(item, app: app)
            {
                return item
            }
        }
        return nil
    }

    private static func isCommand(_ item: NSMenuItem, app: AppDelegate) -> Bool {
        guard !item.isHidden, item.target === app, let action = item.action else { return false }
        let actionName = NSStringFromSelector(action)
        guard !excludedActions.contains(actionName) else { return false }
        return app.validateMenuItem(item)
    }

    private static func commandId(action: Selector, tag: Int, path: String) -> String {
        "\(path)|\(NSStringFromSelector(action))|\(tag)"
    }
}
