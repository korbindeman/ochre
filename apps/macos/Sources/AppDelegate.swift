import AppKit
import Darwin
import SwiftUI

/// Owns the app's single window. The window's content is an AppKit skeleton
/// (`RootView`) hosting SwiftUI chrome islands over the embedded CEF web view.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var normalController: BrowserWindowController!
    private var privateControllers: [Int32: BrowserWindowController] = [:]
    private let commandBar = CommandBarController()

    private static let windowWidthKey = BrowserProfile.defaultsKey("windowWidth")
    private static let windowHeightKey = BrowserProfile.defaultsKey("windowHeight")
    private static let defaultWindowSize = NSSize(width: 1180, height: 760)
    private static let minWindowSize = NSSize(width: 640, height: 480)
    /// The ⌘F find bar. Non-private so `browserFindResultCallback` can reach it.
    let findBar = FindBarController()
    /// KVO on `NSApp.effectiveAppearance` — forwards OS light/dark changes to the
    /// core so `System` appearance tracks the OS live (ADR-0030). Held for the
    /// app's lifetime.
    private var appearanceObserver: NSKeyValueObservation?

    /// URLs handed to us before `makeWindow` finishes (cold launch from another app).
    private var pendingExternalURLs: [URL] = []

    func makeWindow() {
        // Install the menu bar first: it owns every app-level shortcut (⌘T, ⌘R,
        // ⌘[, …) as a key equivalent, so it must exist before the browser view
        // starts receiving key events.
        let appName =
            Bundle.main.object(forInfoDictionaryKey: "CFBundleName") as? String ?? "Browser"
        NSApp.mainMenu = MainMenu.build(appName: appName, actions: self)

        // Appearance (ADR-0030). Apply the persisted preference to the native
        // shell (`NSApp.appearance`) before the window exists so it is created in
        // the right look, forward the current OS appearance so the core themes web
        // content before the first tab hydrates, and watch for OS theme changes so
        // `System` mode tracks the OS live.
        updateAppAppearance()
        forwardSystemAppearance()
        forwardSystemBootTime()
        appearanceObserver = NSApp.observe(\.effectiveAppearance) { [weak self] _, _ in
            MainActor.assumeIsolated { self?.forwardSystemAppearance() }
        }
        let windowSize = Self.loadWindowSize()
        normalController = BrowserWindowController(
            windowId: 0,
            isPrivate: false,
            state: BrowserState.normal
        )
        normalController.makeWindow(size: windowSize)

        commandBar.parentWindow = normalController.window
        findBar.parentWindow = normalController.window
        // Drive the find bar's "n of m" counter from the core's find results.
        browser_set_find_result_callback(browserFindResultCallback)

        // Stream tab/space changes from the core into the observable state that
        // drives the sidebar (ADR-0003). Registered before any tab exists.
        browser_set_event_callback(browserTabEventCallback)

        // Sidebar state (pin / hover / empty-canvas float) drives the AppKit
        // skeleton's animated layout. `RootView` owns those subscriptions — it
        // holds the floating host and its geometry — and binds them in its init
        // (before this initial snapshot pull), so the empty-session auto-float the
        // pull may trigger is observed.

        // Pull the initial snapshot — the core already holds the restored
        // session (or the default empty space), and no event fires until the
        // first mutation. Publishing the restored active tab triggers RootView
        // to hydrate it (lazy session restore, ADR-0013).
        normalController.activate()
        BrowserState.normal.reloadStructure()

        // Vault harness: open the Passwords surface at launch so it can be
        // inspected without driving the menu — `BROWSER_VAULT_DEMO` (seeded, shows
        // the list) or `BROWSER_VAULT_OPEN` (unseeded, shows setup/unlock).
        // Delayed past the launch-time empty-state/session setup so it lands on top.
        let env = ProcessInfo.processInfo.environment
        if env["BROWSER_VAULT_DEMO"] != nil || env["BROWSER_VAULT_OPEN"] != nil {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) {
                BrowserState.shared.isVaultOpen = true
                // `BROWSER_VAULT_EDIT` also opens the editor for the seeded login.
                if env["BROWSER_VAULT_EDIT"] != nil {
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
                        VaultModel.shared.editing = EditorTarget(
                            detail: VaultModel.shared.loginDetail(id: "demo"))
                    }
                }
            }
        }

        // History harness: open the History surface at launch so it can be
        // inspected/screenshotted without driving the ⌘Y menu. Mirrors the vault
        // hook; the surface reads whatever the seeded `history.sqlite` holds.
        if env["BROWSER_HISTORY_OPEN"] != nil {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) {
                BrowserState.shared.openHistory()
            }
        }

        // Command-bar harness: present the ⌘T command bar at launch so its
        // (otherwise transient) glass surface can be inspected/screenshotted
        // without synthesizing a keystroke. Symmetric with the vault demo hook.
        // Pinned so it survives resigning key (a background-launched app never
        // becomes active) without stealing focus, so the panel can be captured
        // by window id while occluded.
        if env["BROWSER_COMMANDBAR_DEMO"] != nil {
            commandBar.keepPresentedForDemo = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) {
                self.commandBar.show()
            }
        }

        if !pendingExternalURLs.isEmpty {
            let urls = pendingExternalURLs
            pendingExternalURLs.removeAll()
            openExternalURLs(urls)
        }
    }

    // MARK: - External URLs

    /// Launch Services hands http/https links here — from Mail, Slack, `open`, etc.
    func application(_ sender: NSApplication, open urls: [URL]) {
        guard normalController != nil else {
            pendingExternalURLs.append(contentsOf: urls)
            return
        }
        openExternalURLs(urls)
    }

    /// Open each web URL in a new tab and bring the window forward.
    private func openExternalURLs(_ urls: [URL]) {
        let webURLs = urls.filter { url in
            guard let scheme = url.scheme?.lowercased() else { return false }
            return scheme == "http" || scheme == "https"
        }
        guard !webURLs.isEmpty else { return }

        NSApp.activate(ignoringOtherApps: true)
        normalController.window?.makeKeyAndOrderFront(nil)

        for url in webURLs {
            openTab(url: url.absoluteString, windowId: 0)
        }
    }

    private var focusedController: BrowserWindowController {
        if let key = NSApp.keyWindow,
            let privateCtl = privateControllers.values.first(where: { $0.window === key })
        {
            return privateCtl
        }
        return normalController
    }

    func didClosePrivateWindow(_ windowId: Int32) {
        privateControllers.removeValue(forKey: windowId)
    }

    @objc func newPrivateWindow() {
        PrivateWindowPrompt.showIfNeeded()
        let windowId = browser_private_window_create()
        let state = BrowserState.register(windowId: windowId, isPrivate: true)
        let size =
            normalController.window?.contentRect(forFrameRect: normalController.window!.frame).size
            ?? Self.defaultWindowSize
        let controller = BrowserWindowController(windowId: windowId, isPrivate: true, state: state)
        privateControllers[windowId] = controller
        controller.makeWindow(size: size)
    }

    func openTab(url: String, windowId: Int32) {
        if windowId == 0 {
            normalController.openTab(url: url)
        } else if let controller = privateControllers[windowId] {
            controller.openTab(url: url)
        }
    }

    private static func loadWindowSize() -> NSSize {
        let defaults = UserDefaults.standard
        let width = defaults.double(forKey: windowWidthKey)
        let height = defaults.double(forKey: windowHeightKey)
        guard width > 0, height > 0 else { return defaultWindowSize }
        return NSSize(
            width: max(minWindowSize.width, width),
            height: max(minWindowSize.height, height)
        )
    }

    /// Persist the windowed content size. Skipped while fullscreen — AppKit owns
    /// that geometry and restores the pre-fullscreen frame on exit.
    private func persistWindowSize() {
        guard let window = normalController?.window, !BrowserState.normal.isFullscreen else {
            return
        }
        let size = window.contentRect(forFrameRect: window.frame).size
        UserDefaults.standard.set(Double(size.width), forKey: Self.windowWidthKey)
        UserDefaults.standard.set(Double(size.height), forKey: Self.windowHeightKey)
    }

    /// Hide (or reveal) the real `NSWindow` traffic-light buttons. Hidden while
    /// windowed — the sidebar draws its own (ADR-0012) — and revealed in
    /// fullscreen, where the OS auto-hiding titlebar owns them. Re-asserted on
    /// key/main changes in case AppKit restores them on its own relayout.
    // MARK: - Focus self-test hooks
    //
    // Minimal accessors the P0-1 focus regression (`--focus-selftest`,
    // `FocusSelfTest.swift`) reaches into to drive and observe focus transitions.
    // Kept here so the private window/rootView/commandBar stay private otherwise.

    var focusTestWindow: NSWindow? { normalController?.window }
    var focusTestRootView: RootView? { normalController?.rootView }
    var focusTestCommandBar: CommandBarController { commandBar }

    // MARK: - Menu actions
    //
    // Wired from `MainMenu`. The menu owns every app-level shortcut as a key
    // equivalent, so `performKeyEquivalent:` intercepts these before the event
    // reaches CEF's web view.

    @objc func toggleCommandBar() {
        URLBarModel.shared.endEditing()
        commandBar.parentWindow = focusedController.window
        commandBar.toggle()
    }

    /// Edit the active tab's address in place (⌘L / clicking the sidebar URL
    /// chip): turn the chip into an inline field in the sidebar (ADR-0014). With
    /// no active tab there is nothing to edit — fall back to the command bar for a
    /// new tab. Reveal a collapsed sidebar first so the field is on-screen.
    @objc func editCurrentURL() {
        guard !BrowserState.shared.isSurfaceOpen else { return }
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
        guard !BrowserState.current.isPrivate else { return }
        BrowserState.current.toggleHistory()
    }

    /// Show/hide the Passwords (vault) page — a native content-area surface
    /// (ADR-0010/0016).
    @objc func showVault() {
        guard !BrowserState.current.isPrivate else { return }
        BrowserState.current.toggleVault()
    }

    /// Show/hide the Settings page — a native content-area surface (ADR-0010).
    @objc func showSettings() {
        BrowserState.current.toggleSettings()
    }

    /// Register this app as the system default http/https handler. When the OS
    /// rejects the programmatic request (unsigned dev builds), fall back to the
    /// System Settings picker.
    @objc func setAsDefaultBrowser() {
        if DefaultBrowser.setAsDefault() {
            return
        }
        let alert = NSAlert()
        alert.messageText = "Set Default Browser"
        alert.informativeText =
            "Choose Browser in System Settings → Desktop & Dock → Default web browser."
        alert.addButton(withTitle: "Open System Settings")
        alert.addButton(withTitle: "Cancel")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        DefaultBrowser.openSystemSettings()
    }

    // Tab lifecycle. `openTab`/`closeTab` are called from the command bar,
    // sidebar, and empty state; `closeActiveTab` backs the ⌘W menu item.
    func closeTab(_ id: Int32) { focusedController.closeTab(id) }

    // Save/organize (from the sidebar's tab & folder context menus).
    func setTabSaved(_ id: Int32, _ saved: Bool) { browser_tab_set_pinned(id, saved) }
    func setTabFavorite(_ id: Int32, _ favorite: Bool) { browser_tab_set_favorite(id, favorite) }
    func restoreTabFavorite(_ id: Int32) { browser_tab_restore_favorite(id) }
    func moveTab(_ id: Int32, toFolder folder: Int32?) {
        browser_tab_move_to_folder(id, folder ?? -1)
    }

    /// Drag-and-drop relocation (see `browser_tab_move`): move `id` into a
    /// section — pinned/loose or a folder — positioned before `before` (nil ⇒
    /// the section's end). A folder destination auto-pins.
    func moveTab(_ id: Int32, favorite: Bool, pinned: Bool, folder: Int32?, before: Int32?) {
        browser_tab_move(id, favorite, pinned, folder ?? -1, before ?? -1)
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

    /// Demote a space to a folder in the active space (its open tabs close).
    func spaceToFolder(_ id: Int32) {
        if confirmDelete("Convert to a folder? This space's open tabs will be closed.") {
            browser_space_to_folder(id)
        }
    }

    @objc func newFolder() {
        if let name = promptName(title: "New Folder", initial: "Folder") {
            name.withCString { browser_create_folder($0) }
        }
    }

    @objc func newFolderFromSelectionMenu() {
        let ids = Array(BrowserState.shared.selectedTabIds).sorted()
        guard ids.count >= 2 else { return }
        createFolderFromTabs(ids)
    }

    func createFolderFromTabs(_ ids: [Int32]) {
        guard ids.count >= 2 else { return }
        if let name = promptName(title: "New Folder", initial: "Folder") {
            name.withCString { cName in
                ids.withUnsafeBufferPointer { buf in
                    _ = browser_create_folder_with_tabs(cName, buf.baseAddress, Int32(buf.count))
                }
            }
            BrowserState.shared.clearTabSelection()
        }
    }

    @objc func clearOpenTabsMenu() { clearOpenTabs() }

    /// Close open tabs except the multi-selection (or the active tab when nothing
    /// is selected) — the sidebar Open → Clear control.
    func clearOpenTabs() {
        let state = BrowserState.shared
        let keep = Array(state.selectedTabIds)
        keep.withUnsafeBufferPointer { buf in
            browser_clear_ephemeral_tabs(buf.baseAddress, Int32(buf.count))
        }
        state.clearTabSelection()
    }

    func closeTabs(_ ids: [Int32]) {
        for id in ids { closeTab(id) }
        BrowserState.shared.clearTabSelection()
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
    /// any open app-UI surface (if up) so the reopened tab is actually visible —
    /// only when a tab was in fact reopened.
    @objc func reopenClosedTab() {
        if browser_tab_reopen_closed() >= 0 {
            BrowserState.shared.closeSurfaces()
        }
    }

    @objc func closeActiveTab(_ sender: Any?) {
        // An open app-UI surface (History / Passwords / Settings) intercepts ⌘W —
        // close the surface, not the tab or window beneath it.
        if BrowserState.shared.isSurfaceOpen {
            BrowserState.shared.closeSurfaces()
            return
        }
        if let active = BrowserState.current.activeTabId {
            focusedController.closeTab(active)
            return
        }
        // No active tab. ⌘W only falls back to closing the window on the genuine
        // no-tab screen: the browser holds no tabs in *any* space and nothing is
        // being composed. `activeTabId == nil` alone isn't enough — an empty space
        // with tabs in other spaces, or the New-Tab command bar open over the empty
        // canvas, both read as nil but aren't "nothing to close", and closing the
        // window there is the premature close we're avoiding. In those cases ⌘W is
        // inert (Escape dismisses the command bar; the space switcher reaches the
        // other tabs). `tabs` is the full cross-space list — the UI's own "is
        // anything left" signal.
        if BrowserState.current.tabs.isEmpty && !commandBar.isVisible {
            focusedController.window?.performClose(nil)
        }
    }

    // Find-in-page (⌘F / ⌘G / ⇧⌘G). The bar owns the search state; these just
    // open it and step matches. Find Next/Previous are gated in validateMenuItem.
    @objc func performFind() { findBar.show() }
    @objc func findNext() { findBar.findNext() }
    @objc func findPrevious() { findBar.findPrevious() }

    @objc func reloadPage() { browser_reload() }
    @objc func forceReload() { browser_reload_ignore_cache() }
    @objc func goBack() { browser_go_back() }
    @objc func goForward() { browser_go_forward() }
    @objc func zoomIn() { browser_zoom_in() }
    @objc func zoomOut() { browser_zoom_out() }
    @objc func actualSize() { browser_zoom_reset() }
    @objc func toggleDevTools() { browser_toggle_dev_tools() }

    // Adblock (ADR-0020). Global on/off and per-site off/on; the core reloads the
    // active tab so the change applies on screen.
    @objc func toggleAdblock() { BrowserState.shared.toggleAdblockGlobal() }
    @objc func toggleAdblockForSite() { BrowserState.shared.toggleAdblockForSite() }
    // Cookie-consent (ADR-0026): its own toggle governing reject + hide.
    @objc func toggleCookieConsent() { BrowserState.shared.toggleCookieConsent() }

    // Appearance (ADR-0030): one Light/Dark/System choice drives both web content
    // (prefers-color-scheme, via the core) and the native chrome (NSApp.appearance).
    @objc func setThemeSystem() { setThemePreference(0) }
    @objc func setThemeLight() { setThemePreference(1) }
    @objc func setThemeDark() { setThemePreference(2) }

    /// Persist the appearance preference in the core (which re-themes open tabs),
    /// then update the native shell's appearance and re-forward the OS appearance —
    /// switching to `System` must set `NSApp.appearance = nil` first so the
    /// re-forwarded value reflects the OS again, not the previously-pinned look.
    private func setThemePreference(_ preference: Int32) {
        browser_theme_set_preference(preference)
        updateAppAppearance()
        forwardSystemAppearance()
    }

    /// Point the native chrome at the chosen appearance: Light/Dark pin it, System
    /// (`nil`) tracks the OS. SwiftUI materials and the traffic lights follow this.
    private func updateAppAppearance() {
        switch browser_theme_preference() {
        case 1: NSApp.appearance = NSAppearance(named: .aqua)
        case 2: NSApp.appearance = NSAppearance(named: .darkAqua)
        default: NSApp.appearance = nil
        }
    }

    /// Tell the core whether the current effective appearance is dark. Only used by
    /// the core in `System` mode; under a pinned Light/Dark preference the value is
    /// ignored, so reading the (pinned) effective appearance here is harmless.
    private func forwardSystemAppearance() {
        let isDark = NSApp.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        browser_theme_set_system_dark(isDark)
    }

    /// Forward the Mac's boot time so the vault can lock after a system restart
    /// (OnMacRestart policy) without conflating that with quitting Browser.
    private func forwardSystemBootTime() {
        var boot = timeval()
        var size = MemoryLayout<timeval>.size
        var mib: [Int32] = [CTL_KERN, KERN_BOOTTIME]
        guard sysctl(&mib, 2, &boot, &size, nil, 0) == 0 else { return }
        browser_settings_set_system_boot_secs(UInt64(boot.tv_sec))
    }

    // Single-window app: both Close Window and Quit run the graceful CEF close
    // (see `windowShouldClose`), which is what ends the process.
    @objc func closeWindow(_ sender: Any?) { focusedController.window?.performClose(nil) }
    @objc func quit(_ sender: Any?) {
        if !browser_request_close_all() {
            NSApp.terminate(nil)
        }
    }

    // Grey out Back/Forward when there's no history to move to; everything else
    // is always available.
    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        switch menuItem.action {
        case #selector(goBack): return browser_can_go_back()
        case #selector(goForward): return browser_can_go_forward()
        case #selector(reopenClosedTab):
            return !BrowserState.current.isPrivate && browser_tab_can_reopen_closed()
        case #selector(showHistory), #selector(showVault):
            return !BrowserState.current.isPrivate
        case #selector(findNext), #selector(findPrevious): return findBar.hasActiveSearch
        case #selector(toggleAdblock):
            // Checkmark reflects the global switch; always available.
            menuItem.state = BrowserState.shared.adblockEnabled ? .on : .off
            return true
        case #selector(toggleAdblockForSite):
            // Checked when blocking is on for the active site; enabled only on a
            // web page and when blocking is globally on.
            let s = BrowserState.shared
            let onWebPage = s.activeTab?.url.hasPrefix("http") ?? false
            menuItem.state = (s.adblockEnabled && !s.adblockSiteOff) ? .on : .off
            return s.adblockEnabled && onWebPage
        case #selector(toggleCookieConsent):
            // Checkmark reflects the cookie-consent switch; always available.
            menuItem.state = BrowserState.shared.consentEnabled ? .on : .off
            return true
        case #selector(setThemeSystem):
            menuItem.state = browser_theme_preference() == 0 ? .on : .off
            return true
        case #selector(setThemeLight):
            menuItem.state = browser_theme_preference() == 1 ? .on : .off
            return true
        case #selector(setThemeDark):
            menuItem.state = browser_theme_preference() == 2 ? .on : .off
            return true
        case #selector(setAsDefaultBrowser):
            menuItem.state = DefaultBrowser.isDefault ? .on : .off
            return !DefaultBrowser.isDefault
        case #selector(newFolderFromSelectionMenu):
            return BrowserState.shared.selectedTabIds.count >= 2
        case #selector(clearOpenTabsMenu):
            let state = BrowserState.shared
            let keep: Set<Int32> =
                state.selectedTabIds.isEmpty
                ? Set(state.activeTabId.map { [$0] } ?? [])
                : state.selectedTabIds
            return state.openTabs.contains { !keep.contains($0.id) }
        default: return true
        }
    }

}
