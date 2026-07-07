import AppKit

/// P0-1 focus-routing regression (ADR-0018). Runs on `--focus-selftest`.
///
/// The one seam a screenshot can't prove is whether keyboard focus routes
/// correctly between CEF's web view and the native chrome. This harness drives a
/// scripted sequence of focus transitions with **synthesized input at the app's
/// own dispatch entry points** — `NSMenu.performKeyEquivalent` for shortcuts,
/// `NSApp.sendEvent` for clicks — and asserts, after each, that the main window's
/// `firstResponder` is the active tab's CEF web view. That is the in-process
/// ground truth for "keys will land in the page"; the core's `[focus] cef
/// on_got_focus` stderr is the corroborating second channel.
///
/// It runs headlessly (no Accessibility grant, no frontmost requirement — the
/// whole reason for staying in-process rather than posting `CGEvent`s; see
/// ADR-0018), prints one `[focus-selftest] …` line per case, and exits non-zero
/// on any failure so `just focus-selftest` is a green/red gate.
@MainActor
final class FocusSelfTest {
    private unowned let app: AppDelegate

    /// A network-free fixture page so the harness never depends on connectivity.
    /// Plain (no autofocus) so page focus is driven only by the transitions under
    /// test, never by the page grabbing it on load.
    private let fixtureURL = "data:text/html,<h1>focus-selftest</h1>"

    /// Time between steps — long enough for CEF to settle a focus change and fire
    /// `on_got_focus`, and for the command bar's dismiss animation to complete.
    private let stepDelay = 0.4

    private var steps: [(name: String, run: () -> Void)] = []
    private var stepIndex = 0
    private var failures = 0

    /// Tab ids captured as the fixture tabs open, used to re-select in the
    /// tab-switch case.
    private var tab1: Int32 = -1
    private var tab2: Int32 = -1

    init(app: AppDelegate) {
        self.app = app
    }

    // ─── Runner ──────────────────────────────────────────────────────────────

    func run() {
        NSLog("[focus-selftest] starting")
        buildSteps()
        // Watchdog: a regression harness must never hang a build. If the step
        // chain hasn't exited within the budget (e.g. the main queue stopped
        // being pumped), fail loudly instead of blocking forever. Never fires on
        // the normal path — finish() calls exit() first.
        DispatchQueue.main.asyncAfter(deadline: .now() + 60) {
            NSLog("[focus-selftest] SUMMARY: TIMEOUT — did not complete")
            fflush(stderr)
            exit(2)
        }
        scheduleNext()
    }

    private func scheduleNext() {
        guard stepIndex < steps.count else {
            finish()
            return
        }
        let step = steps[stepIndex]
        stepIndex += 1
        DispatchQueue.main.asyncAfter(deadline: .now() + stepDelay) {
            MainActor.assumeIsolated {
                step.run()
                self.scheduleNext()
            }
        }
    }

    private func finish() {
        if failures == 0 {
            NSLog("[focus-selftest] SUMMARY: all cases PASS")
        } else {
            NSLog("[focus-selftest] SUMMARY: \(failures) case(s) FAILED")
        }
        // Flush the log channel the harness scrapes before exiting.
        fflush(stderr)
        fflush(stdout)
        exit(failures == 0 ? 0 : 1)
    }

    /// Record a case result. `pass` false increments the failure count and flips
    /// the process exit code.
    private func check(_ name: String, _ pass: Bool, _ detail: String = "") {
        if !pass { failures += 1 }
        let suffix = detail.isEmpty ? "" : " (\(detail))"
        NSLog("[focus-selftest] case=\(name) result=\(pass ? "PASS" : "FAIL")\(suffix)")
    }

    // ─── The observable: does the active tab's page hold keyboard focus? ──────

    /// True when the main window's first responder is the active tab's CEF web
    /// view (a descendant of its container). This is exactly "a keystroke sent to
    /// the window now reaches the page."
    private func pageHasFocus() -> Bool {
        guard let window = app.focusTestWindow,
              let container = app.focusTestRootView?.activeCEFContainer,
              let responder = window.firstResponder as? NSView
        else { return false }
        return responder.isDescendant(of: container)
    }

    private func firstResponderName() -> String {
        app.focusTestWindow?.firstResponder
            .map { String(describing: type(of: $0)) } ?? "nil"
    }

    // ─── Cases ───────────────────────────────────────────────────────────────

    private func buildSteps() {
        // Fixture: two tabs. Opening a tab makes it active and (via set_visible)
        // focuses its page.
        steps.append(("open-tab-1", { self.app.openTab(url: self.fixtureURL) }))
        steps.append(("open-tab-2", {
            self.tab1 = BrowserState.shared.activeTabId ?? -1
            self.app.openTab(url: self.fixtureURL)
        }))

        // ── Case A: tab switch focuses the newly-selected tab's page ──
        // Exercises tabs::set_visible → host.set_focus on the activated tab.
        steps.append(("switch-select-tab-1", {
            self.tab2 = BrowserState.shared.activeTabId ?? -1
            browser_tab_select(self.tab1)
        }))
        steps.append(("assert-A-tab-switch-focus", {
            let onTab1 = BrowserState.shared.activeTabId == self.tab1
            self.check("tab-switch-focuses-page", onTab1 && self.pageHasFocus(),
                       "activeTab=\(BrowserState.shared.activeTabId ?? -1) fr=\(self.firstResponderName())")
        }))

        // ── Case B: a click into the page takes keyboard focus ──
        // Drop focus to the bare window first, then synthesize a click in the CEF
        // view — proving click-to-focus routes into CEF's first-responder path.
        steps.append(("click-defocus-then-click-page", {
            self.app.focusTestWindow?.makeFirstResponder(nil)
            if let window = self.app.focusTestWindow,
               let container = self.app.focusTestRootView?.activeCEFContainer {
                self.click(in: container, window: window)
            }
        }))
        steps.append(("assert-B-click-to-focus", {
            self.check("click-focuses-page", self.pageHasFocus(),
                       "fr=\(self.firstResponderName())")
        }))

        // ── Case C: command bar open (⌘T) then close returns focus to page ──
        steps.append(("cmdbar-open", {
            let routed = self.performShortcut("t", [.command])
            self.check("cmd-t-routes-to-menu", routed && self.app.focusTestCommandBar.isVisible,
                       "routed=\(routed) visible=\(self.app.focusTestCommandBar.isVisible)")
        }))
        steps.append(("cmdbar-close", { self.app.focusTestCommandBar.hide() }))
        steps.append(("assert-C-cmdbar-refocus", {
            self.check("cmdbar-close-refocuses-page", self.pageHasFocus(),
                       "fr=\(self.firstResponderName())")
        }))

        // ── Case D: THE regression for the landed fix ──
        // Reproduce the bare-NSWindow first responder (what a sidebar click
        // leaves behind), then open+close the command bar. Without the
        // browser_focus_active_tab() fix in CommandBar.hide(), focus stays on the
        // bare window and keystrokes fall on the floor; with it, the page refocuses.
        steps.append(("regression-bare-window-precondition", {
            self.app.focusTestWindow?.makeFirstResponder(nil)
            self.check("bare-window-precondition-holds", !self.pageHasFocus(),
                       "fr=\(self.firstResponderName())")
        }))
        steps.append(("regression-cmdbar-open", { _ = self.performShortcut("t", [.command]) }))
        steps.append(("regression-cmdbar-close", { self.app.focusTestCommandBar.hide() }))
        steps.append(("assert-D-refocus-from-bare-window", {
            self.check("cmdbar-close-recovers-focus-from-bare-window", self.pageHasFocus(),
                       "fr=\(self.firstResponderName())")
        }))

        // ── Case E: ⌘L inline URL edit open then dismiss returns focus to page ──
        steps.append(("urlbar-edit-open", {
            let routed = self.performShortcut("l", [.command])
            self.check("cmd-l-routes-to-menu", routed && URLBarModel.shared.isEditing,
                       "routed=\(routed) editing=\(URLBarModel.shared.isEditing)")
        }))
        steps.append(("urlbar-edit-end", { URLBarModel.shared.endEditing() }))
        steps.append(("assert-E-urlbar-refocus", {
            self.check("urlbar-dismiss-refocuses-page", self.pageHasFocus(),
                       "fr=\(self.firstResponderName())")
        }))

        // ── Case F: empty sidebar click dismisses inline URL edit ──
        steps.append(("urlbar-edit-open-for-sidebar-click", {
            let routed = self.performShortcut("l", [.command])
            self.check("cmd-l-opens-urlbar-for-sidebar-click",
                       routed && URLBarModel.shared.isEditing,
                       "routed=\(routed) editing=\(URLBarModel.shared.isEditing)")
        }))
        steps.append(("urlbar-sidebar-empty-click", {
            if let window = self.app.focusTestWindow,
               let sidebar = self.app.focusTestRootView?.focusTestSidebarHost {
                // Lower tab-list chrome — below the URL field, clear of tab rows.
                let point = NSPoint(x: sidebar.bounds.midX, y: sidebar.bounds.height * 0.35)
                self.click(at: point, in: sidebar, window: window)
            }
        }))
        steps.append(("assert-F-sidebar-click-dismisses-urlbar", {
            self.check("sidebar-empty-click-dismisses-urlbar", !URLBarModel.shared.isEditing,
                       "editing=\(URLBarModel.shared.isEditing)")
        }))
        steps.append(("assert-F-sidebar-click-refocuses-page", {
            self.check("sidebar-empty-click-refocuses-page", self.pageHasFocus(),
                       "fr=\(self.firstResponderName())")
        }))
    }

    // ─── Synthesized input ───────────────────────────────────────────────────

    /// Offer a shortcut to the main menu exactly as the OS does on a key press —
    /// this is the `performKeyEquivalent:` path the whole design leans on to
    /// intercept app shortcuts before CEF's web view sees them. Returns whether a
    /// menu item claimed it.
    private func performShortcut(_ chars: String, _ modifiers: NSEvent.ModifierFlags) -> Bool {
        guard let event = NSEvent.keyEvent(
            with: .keyDown,
            location: .zero,
            modifierFlags: modifiers,
            timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: app.focusTestWindow?.windowNumber ?? 0,
            context: nil,
            characters: chars,
            charactersIgnoringModifiers: chars,
            isARepeat: false,
            keyCode: 0
        ) else { return false }
        return NSApp.mainMenu?.performKeyEquivalent(with: event) ?? false
    }

    /// Synthesize a left click at the center of `view`, dispatched through
    /// `NSApp.sendEvent` so it runs real hit-testing and first-responder handling
    /// — into CEF's view for the click-to-focus case.
    private func click(in view: NSView, window: NSWindow) {
        click(at: NSPoint(x: view.bounds.midX, y: view.bounds.midY), in: view, window: window)
    }

    private func click(at point: NSPoint, in view: NSView, window: NSWindow) {
        let inWindow = view.convert(point, to: nil)
        for (type, pressure): (NSEvent.EventType, Float) in [(.leftMouseDown, 1), (.leftMouseUp, 0)] {
            if let event = NSEvent.mouseEvent(
                with: type,
                location: inWindow,
                modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber,
                context: nil,
                eventNumber: 0,
                clickCount: 1,
                pressure: pressure
            ) {
                NSApp.sendEvent(event)
            }
        }
    }
}
