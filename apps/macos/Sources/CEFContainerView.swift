import AppKit

/// Whether the CEF child view is driven by the display-sync loop in `RootView`
/// during a sidebar pin/unpin (presentation-layer bounds, ≤60 Hz) or fills the
/// container on ordinary layout passes.
enum CEFSidebarLayout {
    case normal
    case sidebarTransition
}

/// The `NSView` handed to CEF for a single tab. CEF inserts its GPU-hosted web
/// content as a subview; keep it filling the container on resize. Creation is
/// driven explicitly by `RootView.openNewTab` (via `browser_tab_create`) — the
/// container never creates a browser on its own.
final class CEFContainerView: NSView {
    /// Set by `RootView` for the duration of a sidebar pin/unpin animation.
    static var sidebarLayout: CEFSidebarLayout = .normal

    /// Remove CEF's child view so it deallocates and this tab's browser teardown
    /// runs (CEF finishes destroying a browser only once its host view releases).
    func detachCEFView() {
        for subview in subviews {
            subview.removeFromSuperview()
        }
    }

    override func layout() {
        super.layout()
        guard case .normal = Self.sidebarLayout else { return }
        for subview in subviews {
            subview.frame = bounds
        }
    }

    /// Resize CEF subviews to `size` — called from the sidebar transition sync
    /// loop so Chromium tracks the animated content bounds without a settle jerk.
    func applyCEFSize(_ size: NSSize) {
        let frame = NSRect(origin: .zero, size: size)
        for subview in subviews {
            if subview.frame.width != size.width || subview.frame.height != size.height {
                subview.frame = frame
            }
        }
    }
}
