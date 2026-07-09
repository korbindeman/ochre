import AppKit

/// P1-27 sidebar drag-and-drop regression (ADR-0035). Runs on `--drag-selftest`.
///
/// Drives the drag pipeline at its gesture entry points (`TabDragModel
/// .dragChanged` / `.dragEnded`) with pointer positions derived from the *live
/// reported row frames* — the same geometry a real drag resolves against. That
/// covers frame reporting, slot resolution (reorder, save, folder filing,
/// favorite pin/reorder/unpin, no-op suppression, Esc cancel), and the single
/// `move_tab` commit against the real core. The one layer it cannot drive is
/// AppKit's delivery of `DragGesture` events (Apple's code) — that rides the
/// standing interactive pass, like the rest of the sidebar gestures.
///
/// Headless like `FocusSelfTest` (no Accessibility grant, no frontmost
/// requirement), one `[drag-selftest] …` line per case, non-zero exit on any
/// failure so `just drag-selftest` is a green/red gate.
@MainActor
final class DragSelfTest {
    private unowned let app: AppDelegate

    /// Network-free fixture pages so the harness never depends on connectivity.
    private let fixtureURL = "data:text/html,<h1>drag-selftest</h1>"

    /// Time between steps — long enough for a structure change to round-trip the
    /// core and for SwiftUI to republish row frames.
    private let stepDelay = 0.4

    private var steps: [(name: String, run: () -> Void)] = []
    private var stepIndex = 0
    private var failures = 0

    /// Ids captured as the fixture evolves (the open order after the reorder
    /// case is [reordered, first, second]).
    private var savedTabId: Int32 = -1
    private var favorite1: Int32 = -1
    private var favorite2: Int32 = -1
    private var folderId: Int32 = -1
    private var snapshot: [Int32] = []

    init(app: AppDelegate) {
        self.app = app
        // Synthetic drags span runloop turns with no physical mouse down; the
        // leak-guard watchdog would misread that as a stale drag and end it.
        TabDragModel.watchdogDisabledForTesting = true
    }

    // ─── Runner (FocusSelfTest pattern) ──────────────────────────────────────

    func run() {
        NSLog("[drag-selftest] starting")
        buildSteps()
        DispatchQueue.main.asyncAfter(deadline: .now() + 60) {
            NSLog("[drag-selftest] SUMMARY: TIMEOUT — did not complete")
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
            NSLog("[drag-selftest] SUMMARY: all cases PASS")
        } else {
            NSLog("[drag-selftest] SUMMARY: \(failures) case(s) FAILED")
        }
        fflush(stderr)
        fflush(stdout)
        exit(failures == 0 ? 0 : 1)
    }

    private func check(_ name: String, _ pass: Bool, _ detail: String = "") {
        if !pass { failures += 1 }
        let suffix = detail.isEmpty ? "" : " (\(detail))"
        NSLog("[drag-selftest] case=\(name) result=\(pass ? "PASS" : "FAIL")\(suffix)")
    }

    // ─── Geometry access ─────────────────────────────────────────────────────

    /// The drag model of the sidebar that is actually laid out.
    private var drag: TabDragModel? {
        TabDragModel.liveModels.allObjects.first { !$0.geometry.frames.isEmpty }
    }

    private func rect(_ kind: SidebarDropFrameKind) -> CGRect? {
        drag?.geometry.frames.first { $0.kind == kind }?.rect
    }

    private func newTabRect() -> CGRect? {
        drag?.geometry.frames.first {
            if case .newTab = $0.kind { return true }
            return false
        }?.rect
    }

    /// One full synthetic drag in a single turn: pick up at the tab's own frame,
    /// move to `target`, release. Fine whenever the target's frame exists before
    /// the drag starts (everything except the empty-strip pin zone).
    private func performDrag(_ id: Int32, from source: CGRect?, to target: CGPoint) {
        guard let drag, let source else {
            check("drag-precondition", false, "no laid-out sidebar or source frame for \(id)")
            return
        }
        drag.dragChanged(id, location: CGPoint(x: source.midX, y: source.midY))
        drag.dragChanged(id, location: target)
        drag.dragEnded(location: target)
    }

    private func openIds() -> [Int32] { BrowserState.shared.openTabs.map(\.id) }

    // ─── Cases ───────────────────────────────────────────────────────────────

    private func buildSteps() {
        // Fixture: three open tabs.
        steps.append(("open-tab-1", { self.app.openTab(url: self.fixtureURL, windowId: 0) }))
        steps.append(("open-tab-2", { self.app.openTab(url: self.fixtureURL, windowId: 0) }))
        steps.append(("open-tab-3", { self.app.openTab(url: self.fixtureURL, windowId: 0) }))

        // ── Case A: reorder within Open — drag the last open tab above the first.
        steps.append(("drag-reorder", {
            let open = self.openIds()
            guard open.count == 3, let dst = self.rect(.tab(open[0])) else {
                self.check("reorder-fixture", false, "open=\(open)")
                return
            }
            self.snapshot = [open[2], open[0], open[1]]
            self.performDrag(
                open[2],
                from: self.rect(.tab(open[2])),
                to: CGPoint(x: dst.midX, y: dst.minY + 3)
            )
        }))
        steps.append(("assert-A-reorder", {
            self.check(
                "reorder-within-open", self.openIds() == self.snapshot,
                "open=\(self.openIds()) expected=\(self.snapshot)")
        }))

        // ── Case B: dropping a tab on its own row is a no-op.
        steps.append(("drag-noop", {
            let open = self.openIds()
            self.snapshot = open
            guard let first = open.first, let r = self.rect(.tab(first)) else {
                self.check("noop-fixture", false)
                return
            }
            self.performDrag(first, from: r, to: CGPoint(x: r.midX, y: r.minY + 3))
        }))
        steps.append(("assert-B-noop", {
            self.check(
                "self-drop-is-noop", self.openIds() == self.snapshot,
                "open=\(self.openIds())")
        }))

        // ── Case C: save — drag an open tab to the New Tab row's top half
        // (empty loose-saved section).
        steps.append(("drag-save", {
            let open = self.openIds()
            guard open.count == 3, let newTab = self.newTabRect() else {
                self.check("save-fixture", false)
                return
            }
            self.savedTabId = open[1]
            self.performDrag(
                open[1],
                from: self.rect(.tab(open[1])),
                to: CGPoint(x: newTab.midX, y: newTab.minY + 3)
            )
        }))
        steps.append(("assert-C-save", {
            self.check(
                "drag-to-saved",
                BrowserState.shared.looseSavedTabs.map(\.id) == [self.savedTabId],
                "loose=\(BrowserState.shared.looseSavedTabs.map(\.id))")
        }))

        // ── Case D: file into a folder — drop the saved tab on the folder header.
        steps.append(("create-folder", {
            self.folderId = "Drag Test".withCString { browser_create_folder($0) }
        }))
        steps.append(("drag-into-folder", {
            guard let header = self.rect(.folder(self.folderId)) else {
                self.check("folder-fixture", false, "no folder frame")
                return
            }
            self.performDrag(
                self.savedTabId,
                from: self.rect(.tab(self.savedTabId)),
                to: CGPoint(x: header.midX, y: header.midY)
            )
        }))
        steps.append(("assert-D-folder", {
            self.check(
                "drag-onto-folder-files-tab",
                BrowserState.shared.tabs(inFolder: self.folderId).map(\.id) == [self.savedTabId],
                "folder=\(BrowserState.shared.tabs(inFolder: self.folderId).map(\.id))")
        }))

        // ── Case E: pin to the empty strip. The pin zone only renders mid-drag,
        // so this drag spans two steps: begin, then (zone laid out) drop on it.
        steps.append(("drag-favorite-begin", {
            guard let first = self.openIds().first, let r = self.rect(.tab(first)),
                let drag = self.drag
            else {
                self.check("favorite-fixture", false)
                return
            }
            self.favorite1 = first
            drag.dragChanged(first, location: CGPoint(x: r.midX, y: r.midY))
        }))
        steps.append(("drag-favorite-drop", {
            guard let drag = self.drag, let strip = self.rect(.favoriteStrip) else {
                self.check("favorite-strip-zone-appears", false, "no strip frame mid-drag")
                self.drag?.cancelDrag()
                return
            }
            let p = CGPoint(x: strip.midX, y: strip.midY)
            drag.dragChanged(self.favorite1, location: p)
            drag.dragEnded(location: p)
        }))
        steps.append(("assert-E-favorite", {
            self.check(
                "drag-to-empty-strip-pins",
                BrowserState.shared.favoriteTabs.map(\.id) == [self.favorite1],
                "favorites=\(BrowserState.shared.favoriteTabs.map(\.id))")
        }))

        // ── Case F: pin a second tab at the strip's end (past the last icon).
        steps.append(("drag-favorite-2", {
            guard let first = self.openIds().first, let strip = self.rect(.favoriteStrip)
            else {
                self.check("favorite-2-fixture", false)
                return
            }
            self.favorite2 = first
            self.performDrag(
                first,
                from: self.rect(.tab(first)),
                to: CGPoint(x: strip.maxX - 8, y: strip.midY)
            )
        }))
        steps.append(("assert-F-favorite-2", {
            self.check(
                "drag-to-strip-end-appends",
                BrowserState.shared.favoriteTabs.map(\.id) == [self.favorite1, self.favorite2],
                "favorites=\(BrowserState.shared.favoriteTabs.map(\.id))")
        }))

        // ── Case G: reorder within the strip — drag the second icon left of the
        // first (pointer in the first icon's left half).
        steps.append(("drag-favorite-reorder", {
            guard let anchor = self.rect(.favorite(self.favorite1)) else {
                self.check("favorite-reorder-fixture", false)
                return
            }
            self.performDrag(
                self.favorite2,
                from: self.rect(.favorite(self.favorite2)),
                to: CGPoint(x: anchor.minX + 4, y: anchor.midY)
            )
        }))
        steps.append(("assert-G-favorite-reorder", {
            self.check(
                "strip-reorder",
                BrowserState.shared.favoriteTabs.map(\.id) == [self.favorite2, self.favorite1],
                "favorites=\(BrowserState.shared.favoriteTabs.map(\.id))")
        }))

        // ── Case H: Esc cancels — begin a drag aimed into the list, cancel,
        // nothing moves.
        steps.append(("drag-cancel", {
            self.snapshot = BrowserState.shared.favoriteTabs.map(\.id)
            guard let drag = self.drag, let icon = self.rect(.favorite(self.favorite2)),
                let viewport = self.rect(.viewport)
            else {
                self.check("cancel-fixture", false)
                return
            }
            drag.dragChanged(
                self.favorite2, location: CGPoint(x: icon.midX, y: icon.midY))
            drag.dragChanged(
                self.favorite2, location: CGPoint(x: viewport.midX, y: viewport.midY))
            drag.cancelDrag()
            // The real gesture still fires `onEnded` when the mouse is released
            // after an Esc cancel — it must be swallowed, not committed.
            drag.dragEnded(location: CGPoint(x: viewport.midX, y: viewport.midY))
        }))
        steps.append(("assert-H-cancel", {
            let favorites = BrowserState.shared.favoriteTabs.map(\.id)
            self.check(
                "cancel-leaves-structure-alone",
                favorites == self.snapshot && self.drag?.isDragging != true,
                "favorites=\(favorites) dragging=\(String(describing: self.drag?.isDragging))")
        }))

        // ── Case I: unpin — drag a favorite into the empty area below the rows
        // (appends to Open).
        steps.append(("drag-unpin", {
            guard let viewport = self.rect(.viewport) else {
                self.check("unpin-fixture", false)
                return
            }
            self.performDrag(
                self.favorite1,
                from: self.rect(.favorite(self.favorite1)),
                to: CGPoint(x: viewport.midX, y: viewport.maxY - 40)
            )
        }))
        steps.append(("assert-I-unpin-and-final-state", {
            let state = BrowserState.shared
            let favorites = state.favoriteTabs.map(\.id)
            let open = self.openIds()
            let folder = state.tabs(inFolder: self.folderId).map(\.id)
            self.check(
                "unpin-to-open",
                open == [self.favorite1] && favorites == [self.favorite2]
                    && folder == [self.savedTabId],
                "open=\(open) favorites=\(favorites) folder=\(folder)")
        }))
    }
}
