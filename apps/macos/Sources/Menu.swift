//! The app's main menu bar.
//!
//! Before this the app shipped with no menu bar and piped a handful of
//! shortcuts through a local key monitor in the AppDelegate. macOS routes
//! ⌘-equivalents, the standard application/window items, and the clipboard/undo
//! actions through `NSApp.mainMenu`, so a real menu is both the discoverable
//! home for our shortcuts and the only way to get working Cut/Copy/Paste — those
//! use nil-targeted first-responder selectors that dispatch down the responder
//! chain into CEF's web view.
//!
//! Browser-specific items (navigation, reload, zoom, devtools, sidebar, new tab,
//! close/quit) target the `AppDelegate`; standard app/edit/window items use the
//! system first-responder selectors so AppKit handles them.

import AppKit

@MainActor
enum MainMenu {
    /// Build the full menu bar. `actions` is the target for browser-specific
    /// items; assign the result to `NSApp.mainMenu`.
    static func build(appName: String, actions: AppDelegate) -> NSMenu {
        let mainMenu = NSMenu()
        mainMenu.addItem(appMenu(appName: appName, actions: actions))
        mainMenu.addItem(fileMenu(actions: actions))
        mainMenu.addItem(editMenu())
        mainMenu.addItem(viewMenu(actions: actions))
        mainMenu.addItem(historyMenu(actions: actions))
        mainMenu.addItem(windowMenu())
        return mainMenu
    }

    // MARK: - Application menu

    private static func appMenu(appName: String, actions: AppDelegate) -> NSMenuItem {
        let menu = NSMenu()

        menu.addItem(
            withTitle: "About \(appName)",
            action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
            keyEquivalent: "")
        menu.addItem(.separator())

        let services = NSMenu()
        menu.addItem(withTitle: "Services", action: nil, keyEquivalent: "").submenu = services
        NSApp.servicesMenu = services
        menu.addItem(.separator())

        menu.addItem(
            withTitle: "Hide \(appName)",
            action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        let hideOthers = menu.addItem(
            withTitle: "Hide Others",
            action: #selector(NSApplication.hideOtherApplications(_:)), keyEquivalent: "h")
        hideOthers.keyEquivalentModifierMask = [.command, .option]
        menu.addItem(
            withTitle: "Show All",
            action: #selector(NSApplication.unhideAllApplications(_:)), keyEquivalent: "")
        menu.addItem(.separator())

        // Route Quit through the same CEF-safe close flow as the window's close
        // button — not `NSApp.terminate`, which would exit the process before
        // `browser_shutdown` runs (the message loop returning in main.swift is
        // what drives a clean teardown).
        let quit = menu.addItem(
            withTitle: "Quit \(appName)",
            action: #selector(AppDelegate.quit(_:)), keyEquivalent: "q")
        quit.target = actions

        return submenu(menu)
    }

    // MARK: - File

    private static func fileMenu(actions: AppDelegate) -> NSMenuItem {
        let menu = NSMenu(title: "File")
        // "New Tab" opens the command bar (Arc's ⌘T); entering a URL/search there
        // opens a new tab. "Open Location…" (⌘L) opens it to edit the active
        // tab's address in place. ⌘W closes the active tab (falling back to the
        // window when none are open); ⌘⇧W closes the window.
        target(menu.addItem(withTitle: "New Tab",
                            action: #selector(AppDelegate.toggleCommandBar), keyEquivalent: "t"), actions)
        let newSpace = menu.addItem(withTitle: "New Space",
                            action: #selector(AppDelegate.newSpace), keyEquivalent: "n")
        newSpace.keyEquivalentModifierMask = [.command, .shift]
        target(newSpace, actions)
        target(menu.addItem(withTitle: "New Folder",
                            action: #selector(AppDelegate.newFolder), keyEquivalent: ""), actions)
        target(menu.addItem(withTitle: "Open Location…",
                            action: #selector(AppDelegate.editCurrentURL), keyEquivalent: "l"), actions)
        menu.addItem(.separator())
        target(menu.addItem(withTitle: "Close Tab",
                            action: #selector(AppDelegate.closeActiveTab(_:)), keyEquivalent: "w"), actions)
        let closeWindow = menu.addItem(withTitle: "Close Window",
                            action: #selector(AppDelegate.closeWindow(_:)), keyEquivalent: "w")
        closeWindow.keyEquivalentModifierMask = [.command, .shift]
        target(closeWindow, actions)
        // ⌘⇧T reopens the last closed tab; disabled (and the shortcut inert)
        // when there's nothing to reopen — see `AppDelegate.validateMenuItem`.
        let reopen = menu.addItem(withTitle: "Reopen Closed Tab",
                            action: #selector(AppDelegate.reopenClosedTab), keyEquivalent: "t")
        reopen.keyEquivalentModifierMask = [.command, .shift]
        target(reopen, actions)
        return submenu(menu)
    }

    // MARK: - Edit

    private static func editMenu() -> NSMenuItem {
        let menu = NSMenu(title: "Edit")
        menu.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        let redo = menu.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "z")
        redo.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(.separator())
        menu.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        menu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        menu.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        menu.addItem(withTitle: "Delete", action: #selector(NSText.delete(_:)), keyEquivalent: "")
        menu.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        return submenu(menu)
    }

    // MARK: - View

    private static func viewMenu(actions: AppDelegate) -> NSMenuItem {
        let menu = NSMenu(title: "View")
        target(menu.addItem(withTitle: "Reload Page",
                            action: #selector(AppDelegate.reloadPage), keyEquivalent: "r"), actions)
        let force = menu.addItem(withTitle: "Force Reload",
                                action: #selector(AppDelegate.forceReload), keyEquivalent: "r")
        force.keyEquivalentModifierMask = [.command, .shift]
        target(force, actions)
        menu.addItem(.separator())
        target(menu.addItem(withTitle: "Actual Size",
                            action: #selector(AppDelegate.actualSize), keyEquivalent: "0"), actions)
        target(menu.addItem(withTitle: "Zoom In",
                            action: #selector(AppDelegate.zoomIn), keyEquivalent: "+"), actions)
        target(menu.addItem(withTitle: "Zoom Out",
                            action: #selector(AppDelegate.zoomOut), keyEquivalent: "-"), actions)
        menu.addItem(.separator())
        target(menu.addItem(withTitle: "Toggle Sidebar",
                            action: #selector(AppDelegate.toggleSidebar), keyEquivalent: "s"), actions)
        menu.addItem(.separator())
        // Toggles the DevTools window (Chrome/Safari behaviour). F12 does the same
        // via the core's keyboard handler; this owns the ⌘⌥I equivalent.
        let devtools = menu.addItem(withTitle: "Developer Tools",
                                   action: #selector(AppDelegate.toggleDevTools), keyEquivalent: "i")
        devtools.keyEquivalentModifierMask = [.command, .option]
        target(devtools, actions)
        return submenu(menu)
    }

    // MARK: - History

    private static func historyMenu(actions: AppDelegate) -> NSMenuItem {
        let menu = NSMenu(title: "History")
        // Back/Forward are enabled/disabled by AppDelegate.validateMenuItem.
        target(menu.addItem(withTitle: "Back",
                            action: #selector(AppDelegate.goBack), keyEquivalent: "["), actions)
        target(menu.addItem(withTitle: "Forward",
                            action: #selector(AppDelegate.goForward), keyEquivalent: "]"), actions)
        menu.addItem(.separator())
        target(menu.addItem(withTitle: "Show All History",
                            action: #selector(AppDelegate.showHistory), keyEquivalent: "y"), actions)
        return submenu(menu)
    }

    // MARK: - Window

    private static func windowMenu() -> NSMenuItem {
        let menu = NSMenu(title: "Window")
        menu.addItem(withTitle: "Minimize",
                     action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        menu.addItem(withTitle: "Zoom", action: #selector(NSWindow.performZoom(_:)), keyEquivalent: "")
        menu.addItem(.separator())
        menu.addItem(withTitle: "Bring All to Front",
                     action: #selector(NSApplication.arrangeInFront(_:)), keyEquivalent: "")
        // Let AppKit auto-manage the window list under this menu.
        NSApp.windowsMenu = menu
        return submenu(menu)
    }

    // MARK: - Helpers

    /// Wrap a submenu in the top-level bar item that owns it. The bar item's own
    /// title is ignored by AppKit (the submenu's title is what shows), so the
    /// application menu can pass an untitled `NSMenu` and still read as the app
    /// name.
    private static func submenu(_ menu: NSMenu) -> NSMenuItem {
        let item = NSMenuItem()
        item.submenu = menu
        return item
    }

    @discardableResult
    private static func target(_ item: NSMenuItem, _ target: AnyObject) -> NSMenuItem {
        item.target = target
        return item
    }
}
