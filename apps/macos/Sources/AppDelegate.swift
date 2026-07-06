import AppKit
import Combine
import SwiftUI

/// Owns the app's single window. The window's content is an AppKit skeleton
/// (`RootView`) hosting SwiftUI chrome islands over the embedded CEF web view.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private var window: NSWindow?
    private var rootView: RootView?
    private let commandBar = CommandBarController()
    private var cancellables: Set<AnyCancellable> = []

    func makeWindow() {
        // Install the menu bar first: it owns every app-level shortcut (⌘T, ⌘R,
        // ⌘[, …) as a key equivalent, so it must exist before the browser view
        // starts receiving key events.
        let appName = Bundle.main.object(forInfoDictionaryKey: "CFBundleName") as? String ?? "Browser"
        NSApp.mainMenu = MainMenu.build(appName: appName, actions: self)

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1180, height: 760),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.title = "Browser"
        window.isMovableByWindowBackground = true
        window.delegate = self
        window.center()
        // The app launches with no live browsers — the persisted session (or
        // the empty state) is restored after this, and every CEF browser is
        // created on demand once the window is already visible, so CEF's
        // compositor comes up painting.
        window.makeKeyAndOrderFront(nil)
        let rootView = RootView(frame: window.contentRect(forFrameRect: window.frame))
        window.contentView = rootView
        self.window = window
        self.rootView = rootView

        // The sidebar draws its own traffic lights (ADR-0012); hide the real
        // window buttons while windowed. Fullscreen re-shows them so the OS
        // auto-hiding titlebar can present them (see the fullscreen callbacks).
        setNativeButtonsHidden(true)

        commandBar.parentWindow = window

        // Stream tab/space changes from the core into the observable state that
        // drives the sidebar (ADR-0003). Registered before any tab exists.
        browser_set_event_callback(browserTabEventCallback)
        // Pull the initial snapshot — the core already holds the restored
        // session (or the default empty space), and no event fires until the
        // first mutation. Publishing the restored active tab triggers RootView
        // to hydrate it (lazy session restore, ADR-0013).
        BrowserState.shared.reloadStructure()

        // Sidebar state drives the AppKit skeleton's (animated) layout.
        BrowserState.shared.$isSidebarPinned
            .dropFirst()
            .sink { [weak self] pinned in self?.rootView?.setSidebarPinned(pinned) }
            .store(in: &cancellables)
        BrowserState.shared.$isSidebarRevealed
            .dropFirst()
            .sink { [weak self] revealed in self?.rootView?.setSidebarRevealed(revealed) }
            .store(in: &cancellables)
    }

    /// Hide (or reveal) the real `NSWindow` traffic-light buttons. Hidden while
    /// windowed — the sidebar draws its own (ADR-0012) — and revealed in
    /// fullscreen, where the OS auto-hiding titlebar owns them. Re-asserted on
    /// key/main changes in case AppKit restores them on its own relayout.
    private func setNativeButtonsHidden(_ hidden: Bool) {
        for type: NSWindow.ButtonType in [.closeButton, .miniaturizeButton, .zoomButton] {
            window?.standardWindowButton(type)?.isHidden = hidden
        }
    }

    private func reassertNativeButtons() {
        setNativeButtonsHidden(!BrowserState.shared.isFullscreen)
    }

    // MARK: - Menu actions
    //
    // Wired from `MainMenu`. The menu owns every app-level shortcut as a key
    // equivalent, so `performKeyEquivalent:` intercepts these before the event
    // reaches CEF's web view.

    @objc func toggleCommandBar() {
        // The command bar is the new-tab surface; if the sidebar URL editor is
        // open, dismiss it first so the two entry surfaces never overlap.
        URLBarModel.shared.endEditing()
        commandBar.toggle()
    }

    /// Edit the active tab's address in place (⌘L / clicking the sidebar URL
    /// chip): turn the chip into an inline field in the sidebar (ADR-0014). With
    /// no active tab there is nothing to edit — fall back to the command bar for a
    /// new tab. Reveal a collapsed sidebar first so the field is on-screen.
    @objc func editCurrentURL() {
        guard BrowserState.shared.activeTab != nil else {
            commandBar.toggle()
            return
        }
        if !BrowserState.shared.isSidebarPinned {
            BrowserState.shared.revealSidebar()
        }
        URLBarModel.shared.beginEditing()
    }

    @objc func toggleSidebar() {
        BrowserState.shared.toggleSidebarPinned()
    }

    /// Show/hide the History page (⌘Y) — a native content-area surface, not a
    /// tab (ADR-0010).
    @objc func showHistory() {
        BrowserState.shared.toggleHistory()
    }

    // Tab lifecycle. `openTab`/`closeTab` are called from the command bar,
    // sidebar, and empty state; `closeActiveTab` backs the ⌘W menu item.
    func openTab(url: String) { rootView?.openNewTab(url: url) }
    func closeTab(_ id: Int32) { rootView?.closeTab(id) }

    // Pin/organize (from the sidebar's tab & folder context menus).
    func setTabPinned(_ id: Int32, _ pinned: Bool) { browser_tab_set_pinned(id, pinned) }
    func moveTab(_ id: Int32, toFolder folder: Int32?) {
        browser_tab_move_to_folder(id, folder ?? -1)
    }

    /// Drag-and-drop relocation (see `browser_tab_move`): move `id` into a
    /// section — pinned/loose or a folder — positioned before `before` (nil ⇒
    /// the section's end). A folder destination auto-pins.
    func moveTab(_ id: Int32, pinned: Bool, folder: Int32?, before: Int32?) {
        browser_tab_move(id, pinned, folder ?? -1, before ?? -1)
    }

    // ─── Spaces & folders ──────────────────────────────────────────────────

    @objc func newSpace() {
        if let (name, icon) = promptSpace(title: "New Space", name: "Space", icon: "✨") {
            name.withCString { cName in
                icon.withCString { cIcon in browser_create_space(cName, cIcon) }
            }
        }
    }

    func renameSpace(_ id: Int32) {
        let current = BrowserState.shared.spaces.first { $0.id == id }?.name ?? ""
        if let name = promptName(title: "Rename Space", initial: current) {
            name.withCString { browser_rename_space(id, $0) }
        }
    }

    func changeSpaceIcon(_ id: Int32) {
        let current = BrowserState.shared.spaces.first { $0.id == id }?.icon ?? "✨"
        if let icon = promptIcon(title: "Change Space Icon", initial: current) {
            icon.withCString { browser_set_space_icon(id, $0) }
        }
    }

    func deleteSpace(_ id: Int32) {
        if confirmDelete("Delete this space and close its tabs?") {
            browser_delete_space(id)
        }
    }

    /// Demote a space to a folder in the active space (its ephemeral tabs close).
    func spaceToFolder(_ id: Int32) {
        if confirmDelete("Convert to a folder? This space's unpinned tabs will be closed.") {
            browser_space_to_folder(id)
        }
    }

    @objc func newFolder() {
        if let name = promptName(title: "New Folder", initial: "Folder") {
            name.withCString { browser_create_folder($0) }
        }
    }

    func renameFolder(_ id: Int32) {
        let current = BrowserState.shared.folders.first { $0.id == id }?.name ?? ""
        if let name = promptName(title: "Rename Folder", initial: current) {
            name.withCString { browser_rename_folder(id, $0) }
        }
    }

    /// Delete a folder — its tabs survive (drop to loose pinned), so no confirm.
    func deleteFolder(_ id: Int32) { browser_delete_folder(id) }

    /// Promote a folder to its own space.
    func folderToSpace(_ id: Int32) { browser_folder_to_space(id) }

    /// Prompt for a space's name + emoji icon (icon field first, Arc-style);
    /// nil if cancelled or the name is empty. ⌃⌘Space opens the emoji palette
    /// in the icon field; input is clamped to its first character.
    private func promptSpace(title: String, name: String, icon: String) -> (String, String)? {
        let alert = NSAlert()
        alert.messageText = title
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: "Cancel")
        let container = NSView(frame: NSRect(x: 0, y: 0, width: 240, height: 24))
        let iconField = NSTextField(frame: NSRect(x: 0, y: 0, width: 28, height: 24))
        iconField.stringValue = icon
        iconField.alignment = .center
        let nameField = NSTextField(frame: NSRect(x: 34, y: 0, width: 206, height: 24))
        nameField.stringValue = name
        container.addSubview(iconField)
        container.addSubview(nameField)
        alert.accessoryView = container
        alert.window.initialFirstResponder = nameField
        guard alert.runModal() == .alertFirstButtonReturn else { return nil }
        let chosenName = nameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !chosenName.isEmpty else { return nil }
        return (chosenName, firstGrapheme(of: iconField.stringValue, fallback: icon))
    }

    /// Prompt for a single emoji; nil if cancelled.
    private func promptIcon(title: String, initial: String) -> String? {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = "Press ⌃⌘Space for the emoji picker."
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: "Cancel")
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 60, height: 24))
        field.stringValue = initial
        field.alignment = .center
        alert.accessoryView = field
        alert.window.initialFirstResponder = field
        guard alert.runModal() == .alertFirstButtonReturn else { return nil }
        return firstGrapheme(of: field.stringValue, fallback: initial)
    }

    /// First grapheme cluster of the trimmed input (one emoji, including ZWJ
    /// sequences), or `fallback` when empty — icons are exactly one character.
    private func firstGrapheme(of input: String, fallback: String) -> String {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let first = trimmed.first else { return fallback }
        return String(first)
    }

    /// Prompt for a name with a modal text field; nil if cancelled or empty.
    private func promptName(title: String, initial: String) -> String? {
        let alert = NSAlert()
        alert.messageText = title
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: "Cancel")
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 240, height: 24))
        field.stringValue = initial
        alert.accessoryView = field
        alert.window.initialFirstResponder = field
        guard alert.runModal() == .alertFirstButtonReturn else { return nil }
        let name = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.isEmpty ? nil : name
    }

    /// A destructive-action confirmation; true to proceed.
    private func confirmDelete(_ message: String) -> Bool {
        let alert = NSAlert()
        alert.messageText = message
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Delete")
        alert.addButton(withTitle: "Cancel")
        return alert.runModal() == .alertFirstButtonReturn
    }

    /// Reopen the most recently closed tab (⌘⇧T). The core restores it into its
    /// original space/section and makes it active; `RootView` hydrates it on the
    /// resulting `StructureChanged`, exactly like a session-restored tab. Dismiss
    /// the History surface (if up) so the reopened tab is actually visible — only
    /// when a tab was in fact reopened.
    @objc func reopenClosedTab() {
        if browser_tab_reopen_closed() >= 0 {
            BrowserState.shared.closeHistory()
        }
    }

    @objc func closeActiveTab(_ sender: Any?) {
        // An open app-UI surface (History, …) intercepts ⌘W — close the surface,
        // not the tab or window beneath it.
        if BrowserState.shared.isHistoryOpen {
            BrowserState.shared.closeHistory()
            return
        }
        if let active = BrowserState.shared.activeTabId {
            rootView?.closeTab(active)
        } else {
            // No tabs to close — ⌘W falls back to closing the window, matching
            // Chrome/Safari's ⌘W-on-last behaviour.
            window?.performClose(nil)
        }
    }

    @objc func reloadPage() { browser_reload() }
    @objc func forceReload() { browser_reload_ignore_cache() }
    @objc func goBack() { browser_go_back() }
    @objc func goForward() { browser_go_forward() }
    @objc func zoomIn() { browser_zoom_in() }
    @objc func zoomOut() { browser_zoom_out() }
    @objc func actualSize() { browser_zoom_reset() }
    @objc func toggleDevTools() { browser_toggle_dev_tools() }

    // Single-window app: both Close Window and Quit run the graceful CEF close
    // (see `windowShouldClose`), which is what ends the process.
    @objc func closeWindow(_ sender: Any?) { window?.performClose(nil) }
    @objc func quit(_ sender: Any?) { window?.performClose(nil) }

    // Grey out Back/Forward when there's no history to move to; everything else
    // is always available.
    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        switch menuItem.action {
        case #selector(goBack): return browser_can_go_back()
        case #selector(goForward): return browser_can_go_forward()
        case #selector(reopenClosedTab): return browser_tab_can_reopen_closed()
        default: return true
        }
    }

    // Fullscreen swaps ownership of the lights: windowed, the sidebar draws them
    // and the native buttons stay hidden; fullscreen, the native buttons come
    // back for the OS auto-hiding titlebar and the sidebar's set steps aside
    // (via `isFullscreen`).
    func windowWillEnterFullScreen(_ notification: Notification) {
        BrowserState.shared.isFullscreen = true
        setNativeButtonsHidden(false)
    }

    func windowDidExitFullScreen(_ notification: Notification) {
        BrowserState.shared.isFullscreen = false
        setNativeButtonsHidden(true)
    }

    // AppKit can restore the standard buttons when the window changes key/main
    // state; re-hide them (unless fullscreen) so they never flash back over the
    // sidebar's own lights.
    func windowDidBecomeKey(_ notification: Notification) { reassertNativeButtons() }
    func windowDidResignKey(_ notification: Notification) { reassertNativeButtons() }
    func windowDidBecomeMain(_ notification: Notification) { reassertNativeButtons() }
    func windowDidResignMain(_ notification: Notification) { reassertNativeButtons() }

    // Route the close button through CEF instead of tearing the window down
    // under a live browser: the first close request starts CEF's browser-close
    // flow and is vetoed; CEF re-requests the close once the browser allows it.
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        !browser_request_close_all()
    }

    // Final close: detach every tab's CEF child view so the browsers deallocate
    // — that is what completes their destruction and quits the message loop
    // (`main.swift` then shuts CEF down and the process exits). The view
    // hierarchy outlives the window, so the detach must be explicit.
    func windowWillClose(_ notification: Notification) {
        rootView?.detachAllCEFViews()
        window = nil
        rootView = nil
    }
}
