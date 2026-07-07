import AppKit

/// The `NSView` handed to CEF for a single tab. CEF inserts its GPU-hosted web
/// content as a subview; keep it filling the container on resize. Creation is
/// driven explicitly by `RootView.openNewTab` (via `browser_tab_create`) — the
/// container never creates a browser on its own.
final class CEFContainerView: NSView {
    /// Remove CEF's child view so it deallocates and this tab's browser teardown
    /// runs (CEF finishes destroying a browser only once its host view releases).
    func detachCEFView() {
        for subview in subviews {
            subview.removeFromSuperview()
        }
    }

    // CEF adds its web content as a subview; keep it filling the container on
    // every resize so the page tracks the window.
    override func layout() {
        super.layout()
        for subview in subviews {
            subview.frame = bounds
        }
    }
}
