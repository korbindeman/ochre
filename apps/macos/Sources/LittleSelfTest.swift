import AppKit

/// Little-window regression (P1-45, ADR-0067). Runs on `--little-selftest`.
///
/// Guards the parts a unit test cannot reach: a little window's tab stays out
/// of the sidebar, its content events reach the toolbar, ⌘O moves the *live*
/// page into the main window (the fixture titles itself with a random number,
/// so a reload would change it), and closing a little window removes its tab.
/// One `[little-selftest] …` line per case; non-zero exit on any failure.
@MainActor
final class LittleSelfTest {
    private unowned let app: AppDelegate
    private let fixtureURL =
        "data:text/html,<script>document.title='little-'+Math.random()</script>"
    private let stepDelay = 0.8
    private var steps: [(name: String, run: () -> Void)] = []
    private var stepIndex = 0
    private var failures = 0
    private var littleId: Int32 = -1
    private var titleBeforePromote = ""

    init(app: AppDelegate) {
        self.app = app
    }

    func run() {
        NSLog("[little-selftest] starting")
        buildSteps()
        DispatchQueue.main.asyncAfter(deadline: .now() + 60) {
            NSLog("[little-selftest] SUMMARY: TIMEOUT - did not complete")
            fflush(stderr)
            exit(2)
        }
        scheduleNext()
    }

    private func scheduleNext() {
        guard stepIndex < steps.count else { return finish() }
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
        NSLog(
            failures == 0
                ? "[little-selftest] SUMMARY: all cases PASS"
                : "[little-selftest] SUMMARY: \(failures) case(s) FAILED")
        fflush(stderr)
        exit(failures == 0 ? 0 : 1)
    }

    private func check(_ name: String, _ pass: Bool, _ detail: String = "") {
        if !pass { failures += 1 }
        let suffix = detail.isEmpty ? "" : " (\(detail))"
        NSLog("[little-selftest] case=\(name) result=\(pass ? "PASS" : "FAIL")\(suffix)")
    }

    private var state: BrowserState { BrowserState.normal }

    private func buildSteps() {
        steps.append(("open", { self.app.openLittleWindow(url: self.fixtureURL) }))
        steps.append(("settle", {}))
        steps.append(("opened-outside-sidebar", {
            self.littleId = self.state.littleTabs.first?.id ?? -1
            self.check(
                "opened-outside-sidebar",
                self.state.littleTabs.count == 1 && self.app.littleControllers.count == 1
                    && !self.state.tabs.contains { $0.id == self.littleId },
                "little=\(self.state.littleTabs.map(\.id)) sidebar=\(self.state.tabs.map(\.id))")
        }))
        steps.append(("title-reaches-toolbar", {
            self.titleBeforePromote = self.app.littleControllers.first?.tab?.title ?? ""
            self.check(
                "title-reaches-toolbar", self.titleBeforePromote.hasPrefix("little-"),
                "title=\(self.titleBeforePromote)")
        }))
        steps.append(("promote", { self.app.littleControllers.first?.promote() }))
        steps.append(("promoted-keeps-live-page", {
            let tab = self.state.tabs.first { $0.id == self.littleId }
            self.check(
                "promoted-becomes-active-open-tab",
                self.state.littleTabs.isEmpty && self.app.littleControllers.isEmpty
                    && self.state.activeTabId == self.littleId
                    && self.state.openTabs.contains { $0.id == self.littleId },
                "active=\(String(describing: self.state.activeTabId)) little=\(self.state.littleTabs.count)")
            self.check(
                "promoted-keeps-live-page", tab?.title == self.titleBeforePromote,
                "before=\(self.titleBeforePromote) after=\(tab?.title ?? "nil")")
        }))
        steps.append(("open-second", { self.app.openLittleWindow(url: self.fixtureURL) }))
        steps.append(("close-second", { self.app.littleControllers.first?.close() }))
        steps.append(("closed-removes-tab", {
            self.check(
                "closed-removes-tab",
                self.state.littleTabs.isEmpty && self.app.littleControllers.isEmpty
                    && self.state.tabs.count == 1,
                "little=\(self.state.littleTabs.count) sidebar=\(self.state.tabs.count)")
        }))
    }
}
