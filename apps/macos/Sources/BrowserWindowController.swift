import AppKit
import SwiftUI

/// One browser window — normal or private — with its own chrome state and CEF hosts.
@MainActor
final class BrowserWindowController: NSObject, NSWindowDelegate {
    let windowId: Int32
    let isPrivate: Bool
    let state: BrowserState
    let rootView: RootView
    let commandBar = CommandBarController()
    let findBar = FindBarController()

    private(set) var window: NSWindow?
    private var viewportClickMonitor: Any?

    init(windowId: Int32, isPrivate: Bool, state: BrowserState) {
        self.windowId = windowId
        self.isPrivate = isPrivate
        self.state = state
        self.rootView = RootView(frame: .zero, state: state, isPrivate: isPrivate)
        super.init()
    }

    func makeWindow(size: NSSize) {
        let title = isPrivate ? "Private" : "Browser"
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: size),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.title = title
        // Tab reorder uses SwiftUI `onDrag` in the sidebar body; background drag
        // competes with it at `mouseDown`. Top chrome drags via `WindowDragHandle`.
        window.isMovableByWindowBackground = false
        window.delegate = self
        window.center()

        rootView.frame = window.contentRect(forFrameRect: window.frame)
        window.contentView = rootView
        self.window = window

        commandBar.parentWindow = window
        findBar.parentWindow = window

        setNativeButtonsHidden(true)
        window.makeKeyAndOrderFront(nil)
        activate()

        viewportClickMonitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) {
            [weak self] event in
            guard let self, event.window === self.window else { return event }
            self.rootView.dismissTransientFocusIfViewportClick(event)
            return event
        }

        if isPrivate {
            state.reloadStructure()
            rootView.openNewTab(url: "about:blank")
        }
    }

    func activate() {
        BrowserState.current = state
        browser_set_focused_window(windowId)
    }

    func openTab(url: String) {
        activate()
        rootView.openNewTab(url: url)
    }

    func closeTab(_ id: Int32) {
        if isPrivate {
            browser_private_tab_close(windowId, id)
        } else {
            rootView.closeTab(id)
        }
    }

    private func setNativeButtonsHidden(_ hidden: Bool) {
        for type: NSWindow.ButtonType in [.closeButton, .miniaturizeButton, .zoomButton] {
            window?.standardWindowButton(type)?.isHidden = hidden
        }
    }

    // MARK: - NSWindowDelegate

    func windowDidBecomeKey(_ notification: Notification) { activate() }
    func windowDidBecomeMain(_ notification: Notification) { activate() }

    func windowWillEnterFullScreen(_ notification: Notification) {
        state.isFullscreen = true
        setNativeButtonsHidden(false)
    }

    func windowDidExitFullScreen(_ notification: Notification) {
        state.isFullscreen = false
        setNativeButtonsHidden(true)
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        if isPrivate {
            return !browser_request_close_window(windowId)
        }
        // Single-window app: closing the normal window quits gracefully (⌘Q path).
        if !browser_request_close_all() {
            return true
        }
        return false
    }

    func windowWillClose(_ notification: Notification) {
        if let viewportClickMonitor {
            NSEvent.removeMonitor(viewportClickMonitor)
            self.viewportClickMonitor = nil
        }
        if isPrivate {
            browser_private_destroy_window(windowId)
            (NSApp.delegate as? AppDelegate)?.didClosePrivateWindow(windowId)
        } else {
            persistWindowSizeIfNormal()
            browser_settings_lock_vault_on_shutdown()
        }
        rootView.detachAllCEFViews()
        BrowserState.unregister(windowId: windowId)
        window = nil
    }

    private func persistWindowSizeIfNormal() {
        guard !isPrivate, let window, !state.isFullscreen else { return }
        let size = window.contentRect(forFrameRect: window.frame).size
        UserDefaults.standard.set(
            Double(size.width), forKey: BrowserProfile.defaultsKey("windowWidth"))
        UserDefaults.standard.set(
            Double(size.height), forKey: BrowserProfile.defaultsKey("windowHeight"))
    }
}
