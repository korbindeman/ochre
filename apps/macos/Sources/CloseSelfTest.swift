import AppKit

/// P1-30 ⌘W close-path regression (ADR-0037). Runs on `--close-selftest`.
///
/// The invariant under guard: **closing or suspending a tab must never close
/// the window** - CEF's `DoClose` default sends `performClose:` to the
/// browser's parent `NSWindow` (the app window) whenever a tab's view is still
/// attached when its close pipeline lands, and only ADR-0037's embedded-tab
/// suppression stands between a routine ⌘W and a vanished window. The riskiest
/// path is ⌘W on an active **saved** tab (`hide_saved_tab`): its view detach
/// rides the async `TabSuspended` hop, so before ADR-0037 it raced CEF and
/// sometimes lost. The harness drives ⌘W through `NSMenu.performKeyEquivalent`
/// (the real dispatch path) and asserts, with deliberate settling delays that
/// give CEF's close pipeline time to land a stray `performClose:`, that the
/// window survives an ordinary-tab close and a saved-tab suspend - and that
/// ⌘W on the empty canvas *does* close the window (Arc semantics), which the
/// final case observes via `willCloseNotification`. Further cases guard
/// ADR-0038 and ADR-0056. ⌘W on an active **favorite** deselects it and
/// returns it to its pinned URL. The icon and loaded tab must persist.
///
/// Headless like `FocusSelfTest` (no Accessibility grant, no frontmost
/// requirement), one `[close-selftest] …` line per case, non-zero exit on any
/// failure so `just close-selftest` is a green/red gate.
@MainActor
final class CloseSelfTest {
    private unowned let app: AppDelegate

    /// Network-free fixture pages so the harness never depends on connectivity.
    private let fixtureURL = "data:text/html,<h1>close-selftest</h1>"
    private let temporaryURL = "data:text/html,<h1>temporary-navigation</h1>"

    /// Time between steps - long enough for a structure change to round-trip
    /// the core *and* for CEF's async browser-close pipeline (renderer
    /// round-trip → `DoClose`) to land, since a late `performClose:` is exactly
    /// the regression this harness exists to catch.
    private let stepDelay = 0.6

    private var steps: [(name: String, run: () -> Void)] = []
    private var stepIndex = 0
    private var failures = 0
    /// Set by the final case: it finishes asynchronously (via the window's
    /// `willCloseNotification` or its own timeout), so the runner must not
    /// auto-finish when the step list drains.
    private var holdFinish = false

    /// Ids captured as the fixture tabs open.
    private var ordinaryTab: Int32 = -1
    private var savedTab: Int32 = -1
    private var favoriteTab: Int32 = -1

    /// Final-case bookkeeping: the `willCloseNotification` observer and the
    /// once-guard shared by its two completion paths (closed vs. timed out).
    private var willCloseToken: NSObjectProtocol?
    private var closeCaseDone = false

    init(app: AppDelegate) {
        self.app = app
    }

    // ─── Runner (FocusSelfTest pattern) ──────────────────────────────────────

    func run() {
        NSLog("[close-selftest] starting")
        buildSteps()
        DispatchQueue.main.asyncAfter(deadline: .now() + 60) {
            NSLog("[close-selftest] SUMMARY: TIMEOUT - did not complete")
            fflush(stderr)
            exit(2)
        }
        scheduleNext()
    }

    private func scheduleNext() {
        guard stepIndex < steps.count else {
            if !holdFinish { finish() }
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
            NSLog("[close-selftest] SUMMARY: all cases PASS")
        } else {
            NSLog("[close-selftest] SUMMARY: \(failures) case(s) FAILED")
        }
        fflush(stderr)
        fflush(stdout)
        exit(failures == 0 ? 0 : 1)
    }

    private func check(_ name: String, _ pass: Bool, _ detail: String = "") {
        if !pass { failures += 1 }
        let suffix = detail.isEmpty ? "" : " (\(detail))"
        NSLog("[close-selftest] case=\(name) result=\(pass ? "PASS" : "FAIL")\(suffix)")
    }

    // ─── Synthesized input ───────────────────────────────────────────────────

    /// Offer ⌘W to the main menu exactly as the OS does on a key press - the
    /// same `performKeyEquivalent:` dispatch a user's ⌘W takes into
    /// `AppDelegate.closeActiveTab`.
    private func performCmdW() -> Bool {
        guard let event = NSEvent.keyEvent(
            with: .keyDown,
            location: .zero,
            modifierFlags: [.command],
            timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: app.focusTestWindow?.windowNumber ?? 0,
            context: nil,
            characters: "w",
            charactersIgnoringModifiers: "w",
            isARepeat: false,
            keyCode: 13
        ) else { return false }
        return NSApp.mainMenu?.performKeyEquivalent(with: event) ?? false
    }

    private var windowIsOpen: Bool {
        app.focusTestWindow?.isVisible ?? false
    }

    private var state: BrowserState { BrowserState.shared }

    // ─── Cases ───────────────────────────────────────────────────────────────

    private func buildSteps() {
        // Fixture: two open tabs; the second becomes a saved (pinned) tab.
        steps.append(("open-tab-1", {
            self.app.openTab(url: self.fixtureURL, windowId: 0)
        }))
        steps.append(("open-tab-2", {
            self.ordinaryTab = self.state.openTabs.first?.id ?? -1
            self.app.openTab(url: self.fixtureURL, windowId: 0)
        }))
        steps.append(("save-tab-2", {
            self.savedTab = self.state.openTabs.map(\.id).first { $0 != self.ordinaryTab } ?? -1
            guard self.savedTab >= 0, self.ordinaryTab >= 0 else {
                self.check("fixture", false,
                           "open=\(self.state.openTabs.map(\.id))")
                return
            }
            self.app.setTabPinned(self.savedTab, true)
        }))

        // ── Case A: ⌘W on an active ordinary tab closes the tab, not the window.
        steps.append(("select-ordinary", { self.state.selectTab(self.ordinaryTab) }))
        steps.append(("cmdw-ordinary", {
            let routed = self.performCmdW()
            self.check("cmdw-ordinary-routes-to-menu", routed, "routed=\(routed)")
        }))
        steps.append(("assert-A", {
            let gone = !self.state.tabs.contains { $0.id == self.ordinaryTab }
            self.check("ordinary-close-removes-tab-keeps-window",
                       gone && self.windowIsOpen,
                       "gone=\(gone) window=\(self.windowIsOpen)")
        }))

        // ── Case B: ⌘W on an active saved tab returns it home and suspends it.
        // The regression case: the suspend closes the CEF browser while its view
        // is still attached, so a `DoClose` regression `performClose:`s the
        // window here. The extra settle step gives a stray close time to land.
        steps.append(("select-saved", { self.state.selectTab(self.savedTab) }))
        steps.append(("navigate-saved-temporarily", {
            self.temporaryURL.withCString { browser_navigate($0) }
        }))
        steps.append(("cmdw-saved", {
            let tab = self.state.tabs.first { $0.id == self.savedTab }
            let activeAndAway = self.state.activeTabId == self.savedTab
                && tab?.savedURL == self.fixtureURL
                && tab?.url == self.temporaryURL
            self.check("saved-tab-temporary-navigation-precondition", activeAndAway,
                       "active=\(String(describing: self.state.activeTabId)) "
                           + "saved=\(tab?.savedURL ?? "nil") current=\(tab?.url ?? "nil")")
            _ = self.performCmdW()
        }))
        steps.append(("assert-B", {
            let keptAtHome = self.state.tabs.contains {
                $0.id == self.savedTab && $0.pinned
                    && $0.savedURL == self.fixtureURL && $0.url == self.fixtureURL
            }
            let emptyCanvas = self.state.activeTabId == nil
            self.check("saved-close-suspends-to-empty-canvas",
                       keptAtHome && emptyCanvas && self.windowIsOpen,
                       "home=\(keptAtHome) empty=\(emptyCanvas) window=\(self.windowIsOpen)")
        }))
        steps.append(("assert-B-window-survives-cef-close", {
            self.check("window-survives-late-cef-close", self.windowIsOpen,
                       "window=\(self.windowIsOpen)")
        }))

        // ── Case D: ⌘W on an active favorite returns it home and keeps the icon.
        // Favorites are permanent. Close drops the active pointer to the canvas,
        // restores the pinned URL, and preserves the browser and strip icon.
        // (Runs before Case C, which closes the window.)
        steps.append(("open-favorite", {
            self.app.openTab(url: self.fixtureURL, windowId: 0)
        }))
        steps.append(("pin-favorite", {
            self.favoriteTab = self.state.openTabs.first?.id ?? -1
            guard self.favoriteTab >= 0 else {
                self.check("favorite-fixture", false,
                           "open=\(self.state.openTabs.map(\.id))")
                return
            }
            self.app.setTabFavorite(self.favoriteTab, true)
        }))
        steps.append(("select-favorite", { self.state.selectTab(self.favoriteTab) }))
        steps.append(("navigate-favorite-temporarily", {
            self.temporaryURL.withCString { browser_navigate($0) }
        }))
        steps.append(("cmdw-favorite", {
            let tab = self.state.favoriteTabs.first { $0.id == self.favoriteTab }
            let activeAndAway = self.state.activeTabId == self.favoriteTab
                && tab?.savedURL == self.fixtureURL
                && tab?.url == self.temporaryURL
            self.check("favorite-temporary-navigation-precondition", activeAndAway,
                       "active=\(String(describing: self.state.activeTabId)) "
                           + "saved=\(tab?.savedURL ?? "nil") current=\(tab?.url ?? "nil")")
            _ = self.performCmdW()
        }))
        steps.append(("assert-D", {
            let iconStaysAtHome = self.state.favoriteTabs.contains {
                $0.id == self.favoriteTab
                    && $0.savedURL == self.fixtureURL && $0.url == self.fixtureURL
            }
            let deselected = self.state.activeTabId != self.favoriteTab
            self.check("favorite-close-deselects-keeps-icon",
                       iconStaysAtHome && deselected && self.windowIsOpen,
                       "home=\(iconStaysAtHome) deselected=\(deselected) "
                           + "window=\(self.windowIsOpen)")
        }))

        // ── Case C: ⌘W on the empty canvas closes the window (Arc semantics),
        // even with the saved tab still in the sidebar. The close may complete
        // asynchronously (CEF drains browsers before re-requesting it), so pass
        // on `willCloseNotification` and fail on a timeout.
        steps.append(("cmdw-empty-canvas", {
            self.holdFinish = true
            guard let window = self.app.focusTestWindow else {
                self.check("cmdw-empty-closes-window", false, "no window")
                self.finish()
                return
            }
            // Complete synchronously: the observer fires on the main thread
            // inside `performClose:`, and the app may exit (message loop ends
            // once the window closes) before a deferred hop could run.
            self.willCloseToken = NotificationCenter.default.addObserver(
                forName: NSWindow.willCloseNotification, object: window, queue: nil
            ) { _ in
                MainActor.assumeIsolated { self.completeCloseCase(closed: true) }
            }
            _ = self.performCmdW()
            DispatchQueue.main.asyncAfter(deadline: .now() + 3.0) {
                MainActor.assumeIsolated { self.completeCloseCase(closed: false) }
            }
        }))
    }

    private func completeCloseCase(closed: Bool) {
        guard !closeCaseDone else { return }
        closeCaseDone = true
        if let willCloseToken {
            NotificationCenter.default.removeObserver(willCloseToken)
        }
        self.check("cmdw-empty-closes-window", closed, closed ? "" : "window did not close")
        finish()
    }
}
