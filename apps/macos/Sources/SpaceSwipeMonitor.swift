import AppKit

/// Two-finger horizontal trackpad swipes switch spaces (Arc-style), wired through
/// the same `browser_space_select_adjacent` path as ⌘⌥← / ⌘⌥→.
///
/// A local `scrollWheel` monitor sees events before CEF or SwiftUI dispatch them,
/// so a qualifying horizontal gesture is consumed instead of scrolling the page.
/// Vertical scrolls pass through untouched.
@MainActor
final class SpaceSwipeMonitor {
    private weak var window: NSWindow?
    private let isPrivate: Bool
    private let commandBar: CommandBarController
    private let findBar: FindBarController

    private var monitor: Any?

    private var accumulatedX: CGFloat = 0
    private var accumulatedY: CGFloat = 0
    private var isHorizontal = false
    private var isTracking = false
    private var suppressMomentum = false

    /// Accumulated horizontal delta required to commit a space change.
    private let switchThreshold: CGFloat = 36
    /// Horizontal must dominate vertical by this ratio before we claim the gesture.
    private let horizontalDominanceRatio: CGFloat = 1.35

    init(
        isPrivate: Bool,
        commandBar: CommandBarController,
        findBar: FindBarController
    ) {
        self.isPrivate = isPrivate
        self.commandBar = commandBar
        self.findBar = findBar
    }

    func install(on window: NSWindow) {
        uninstall()
        self.window = window
        monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
            guard let self else { return event }
            return self.handle(event)
        }
    }

    func uninstall() {
        if let monitor {
            NSEvent.removeMonitor(monitor)
            self.monitor = nil
        }
        resetGesture()
        suppressMomentum = false
    }

    private func handle(_ event: NSEvent) -> NSEvent? {
        guard !isPrivate else { return event }
        guard event.window === window else { return event }
        guard BrowserState.shared.spaces.count > 1 else { return event }
        if commandBar.isVisible || findBar.isVisible { return event }
        if event.modifierFlags.contains(.shift) { return event }

        // Trackpad gestures carry phase info; ignore legacy mouse-wheel ticks.
        let phase = event.phase
        let momentum = event.momentumPhase
        guard phase != [] || momentum != [] else { return event }

        if momentum != [] {
            if momentum == .ended {
                suppressMomentum = false
            }
            return suppressMomentum ? nil : event
        }

        if phase == .began || phase == .mayBegin {
            resetGesture()
            isTracking = true
        }

        accumulatedX += event.scrollingDeltaX
        accumulatedY += event.scrollingDeltaY

        if abs(accumulatedX) > 6 || abs(accumulatedY) > 6 {
            isHorizontal =
                abs(accumulatedX) > abs(accumulatedY) * horizontalDominanceRatio
        }

        if phase == .ended || phase == .cancelled {
            defer { resetGesture() }
            guard isHorizontal, abs(accumulatedX) >= switchThreshold else { return event }

            // Positive deltaX = fingers swiped left = next space (matches the
            // switcher row and ⌘⌥→).
            let delta: Int32 = accumulatedX > 0 ? 1 : -1
            BrowserState.shared.closeSurfaces()
            _ = browser_space_select_adjacent(delta)
            suppressMomentum = true
            return nil
        }

        if isHorizontal && isTracking {
            return nil
        }

        return event
    }

    private func resetGesture() {
        accumulatedX = 0
        accumulatedY = 0
        isHorizontal = false
        isTracking = false
    }
}
