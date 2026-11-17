import AppKit

/// New-tab creation regression. Runs on `--newtab-selftest`.
///
/// Proves the end-to-end create path (not just ⌘T opening the command bar):
/// 1. Direct `openTab` (FFI → CEF create → register → host view)
/// 2. `Navigation.open` with `.newTab` route (command-bar commit path)
/// 3. Structure snapshot reflects both tabs; active tab is the last created
@MainActor
final class NewTabSelfTest {
    private unowned let app: AppDelegate
    private let fixtureURL = "data:text/html,<h1>newtab-selftest</h1>"
    private let stepDelay = 0.35

    private var steps: [(name: String, run: () -> Void)] = []
    private var stepIndex = 0
    private var failures = 0
    private var beforeCount = 0
    private var afterDirect: Int32 = -1

    init(app: AppDelegate) {
        self.app = app
    }

    func run() {
        NSLog("[newtab-selftest] starting")
        buildSteps()
        DispatchQueue.main.asyncAfter(deadline: .now() + 45) {
            NSLog("[newtab-selftest] SUMMARY: TIMEOUT")
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
            NSLog("[newtab-selftest] SUMMARY: all cases PASS")
        } else {
            NSLog("[newtab-selftest] SUMMARY: \(failures) case(s) FAILED")
        }
        fflush(stderr)
        fflush(stdout)
        exit(failures == 0 ? 0 : 1)
    }

    private func check(_ name: String, _ pass: Bool, _ detail: String = "") {
        if !pass { failures += 1 }
        let suffix = detail.isEmpty ? "" : " (\(detail))"
        NSLog("[newtab-selftest] case=\(name) result=\(pass ? "PASS" : "FAIL")\(suffix)")
    }

    private func buildSteps() {
        steps.append(("snapshot-before", {
            self.beforeCount = BrowserState.shared.tabs.count
            NSLog("[newtab-selftest] before tabs=\(self.beforeCount)")
        }))

        steps.append(("direct-openTab", {
            self.app.openTab(url: self.fixtureURL, windowId: 0)
        }))
        steps.append(("assert-direct-create", {
            let state = BrowserState.shared
            let count = state.tabs.count
            let active = state.activeTabId
            let hasView = self.app.focusTestRootView?.activeCEFContainer != nil
            let pass = count == self.beforeCount + 1 && active != nil && hasView
            self.afterDirect = active ?? -1
            self.check(
                "direct-openTab-creates-tab",
                pass,
                "count=\(count) expected=\(self.beforeCount + 1) active=\(active ?? -1) hasView=\(hasView)"
            )
        }))

        steps.append(("navigation-open-newTab", {
            Navigation.route = .newTab
            Navigation.open("https://example.com/")
        }))
        steps.append(("assert-navigation-create", {
            let state = BrowserState.shared
            let count = state.tabs.count
            let active = state.activeTabId
            let url = state.activeTab?.url ?? ""
            let hasView = self.app.focusTestRootView?.activeCEFContainer != nil
            let pass =
                count == self.beforeCount + 2
                && active != nil
                && active != self.afterDirect
                && url.contains("example.com")
                && hasView
            self.check(
                "Navigation.open-newTab-creates-tab",
                pass,
                "count=\(count) expected=\(self.beforeCount + 2) active=\(active ?? -1) url=\(url) hasView=\(hasView)"
            )
        }))

        steps.append(("cmd-t-then-commit", {
            Navigation.route = .newTab
            _ = self.performShortcut("t", [.command])
        }))
        steps.append(("cmd-t-assert-visible", {
            let visible = self.app.focusTestCommandBar.isVisible
            self.check("cmd-t-opens-command-bar", visible, "visible=\(visible)")
        }))
        steps.append(("cmd-t-commit-via-suggestions", {
            // Simulate typing + return the way CommandBarKeys does.
            let bar = self.app.focusTestCommandBar
            bar.suggestions.prime(query: "https://example.org/")
            Navigation.open(bar.suggestions.commitQuery)
            bar.hide()
        }))
        steps.append(("assert-cmd-t-commit", {
            let state = BrowserState.shared
            let count = state.tabs.count
            let url = state.activeTab?.url ?? ""
            let pass = count == self.beforeCount + 3 && url.contains("example.org")
            self.check(
                "cmd-t-commit-creates-tab",
                pass,
                "count=\(count) expected=\(self.beforeCount + 3) url=\(url)"
            )
        }))
    }

    private func performShortcut(_ key: String, _ modifiers: NSEvent.ModifierFlags) -> Bool {
        guard let event = NSEvent.keyEvent(
            with: .keyDown,
            location: .zero,
            modifierFlags: modifiers,
            timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: 0,
            context: nil,
            characters: key,
            charactersIgnoringModifiers: key,
            isARepeat: false,
            keyCode: 0
        ) else { return false }
        return NSApp.mainMenu?.performKeyEquivalent(with: event) ?? false
    }
}
