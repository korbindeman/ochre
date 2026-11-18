import AppKit
import Combine
import SwiftUI

/// One browser window - normal or private - with its own chrome state and CEF hosts.
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
    private var spaceSwipeMonitor: SpaceSwipeMonitor?
    /// True when *this* controller entered OS fullscreen because content
    /// requested HTML5 fullscreen - so we exit OS fullscreen when content does.
    /// False if the window was already fullscreen (green button) when video
    /// fullscreen started.
    private var enteredWindowFullscreenForContent = false
    private var cancellables: Set<AnyCancellable> = []

    init(windowId: Int32, isPrivate: Bool, state: BrowserState) {
        self.windowId = windowId
        self.isPrivate = isPrivate
        self.state = state
        self.rootView = RootView(frame: .zero, state: state, isPrivate: isPrivate)
        super.init()
        // Alloy style: CEF only resizes content to the browser area. The host
        // must enter/exit real macOS window fullscreen for video/HTML5 fullscreen.
        state.$isContentFullscreen
            .dropFirst()
            .removeDuplicates()
            .receive(on: RunLoop.main)
            .sink { [weak self] want in
                self?.syncWindowFullscreen(forContent: want)
            }
            .store(in: &cancellables)
    }

    func makeWindow(size: NSSize) {
        let title = isPrivate ? "Private" : Product.displayName
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: size),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.title = title
        // Tab reorder uses a drag gesture in the sidebar body (ADR-0035);
        // background drag competes with it at `mouseDown`. Top chrome drags via
        // `WindowDragHandle`.
        window.isMovableByWindowBackground = false
        window.collectionBehavior.insert(.fullScreenPrimary)
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

        let swipeMonitor = SpaceSwipeMonitor(
            isPrivate: isPrivate,
            rootView: rootView,
            commandBar: commandBar,
            findBar: findBar
        )
        swipeMonitor.install(on: window)
        spaceSwipeMonitor = swipeMonitor

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

    func deleteTab(_ id: Int32) {
        if isPrivate {
            browser_private_tab_close(windowId, id)
        } else {
            rootView.deleteTab(id)
        }
    }

    private func setNativeButtonsHidden(_ hidden: Bool) {
        for type: NSWindow.ButtonType in [.closeButton, .miniaturizeButton, .zoomButton] {
            window?.standardWindowButton(type)?.isHidden = hidden
        }
    }

    /// Drive OS window fullscreen to match content (HTML5) fullscreen.
    private func syncWindowFullscreen(forContent want: Bool) {
        guard let window else { return }
        let isWinFS = window.styleMask.contains(.fullScreen)
        if want {
            if !isWinFS {
                enteredWindowFullscreenForContent = true
                window.toggleFullScreen(nil)
            } else {
                enteredWindowFullscreenForContent = false
            }
        } else {
            if enteredWindowFullscreenForContent, isWinFS {
                window.toggleFullScreen(nil)
            }
            enteredWindowFullscreenForContent = false
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
        // User left OS fullscreen (green button / swipe) while content is still
        // in browser fullscreen - tell CEF to exit content fullscreen too.
        if state.isContentFullscreen {
            enteredWindowFullscreenForContent = false
            browser_exit_fullscreen()
        }
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        NSLog("[window] windowShouldClose windowId=\(windowId)")
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
        spaceSwipeMonitor?.uninstall()
        spaceSwipeMonitor = nil
        OverlayMachine.windowClosed(windowId)
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
