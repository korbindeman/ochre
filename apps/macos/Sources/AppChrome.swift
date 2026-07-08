import AppKit
import SwiftUI

/// Dismiss transient native chrome so a click into the web viewport returns
/// focus to the page — URL-bar editing, tab multi-select, and similar.
@MainActor
enum TransientChrome {
    static func dismiss(focusPage: Bool = true) {
        let wasEditing = URLBarModel.shared.isEditing
        URLBarModel.shared.endEditing()
        BrowserState.shared.clearTabSelection()
        if focusPage && !wasEditing {
            browser_focus_active_tab()
        }
    }
}

/// `NSHostingView` that passes clicks through transparent chrome to views below.
/// Interactive SwiftUI descendants still receive hits; empty chrome does not.
final class PassthroughHostingView: NSHostingView<AnyView> {
    override func hitTest(_ point: NSPoint) -> NSView? {
        guard bounds.contains(point) else { return nil }
        guard let hit = super.hitTest(point) else { return nil }
        if hit === self { return nil }
        return hit
    }
}

/// The in-page address bar overlay. CEF draws behind the strip with
/// `clipsToBounds` off, so transparent margins must pass clicks through — but
/// SwiftUI often resolves the pill and its flanking controls to the hosting view
/// itself (no AppKit child), which `PassthroughHostingView` would treat as empty
/// chrome. Claim hits inside the centered control cluster so taps reach SwiftUI.
final class ViewportURLBarHostingView: NSHostingView<AnyView> {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func hitTest(_ point: NSPoint) -> NSView? {
        guard bounds.contains(point) else { return nil }
        if let hit = super.hitTest(point), hit !== self {
            return hit
        }
        return interactiveClusterContains(point) ? self : nil
    }

    /// The pill + optional vault/shield capsules (`URLBar.viewportChrome`).
    private func interactiveClusterContains(_ point: NSPoint) -> Bool {
        let xPad: CGFloat = 24
        let spacing: CGFloat = 8
        let pillH = ViewportURLBarMetrics.pillHeight
        let clusterWidth = min(
            ViewportURLBarMetrics.pillMaxWidth + 2 * pillH + 2 * spacing,
            bounds.width - 2 * xPad
        )
        let cluster = NSRect(
            x: (bounds.width - clusterWidth) / 2,
            y: 0,
            width: clusterWidth,
            height: bounds.height
        )
        return cluster.contains(point)
    }
}
