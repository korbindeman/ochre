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
///
/// When the viewport URL bar is on, the container is shorter than the CEF browser
/// (the viewport starts below the bar) but the CEF child is laid out taller and
/// `clipsToBounds` is off so page content can scroll up behind the transparent
/// chrome strip.
final class CEFContainerView: NSView {
    /// Set by `RootView` for the duration of a sidebar pin/unpin animation.
    static var sidebarLayout: CEFSidebarLayout = .normal
    /// Top chrome height when the viewport URL bar is shown; zero otherwise.
    static var viewportChromeInset: CGFloat = 0

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
        applySubviewFrames(browserSize: nil)
    }

    /// Resize CEF subviews — called from the sidebar transition sync loop so
    /// Chromium tracks the animated content bounds without a settle jerk.
    /// `size` is the full browser size (including the scroll-behind region).
    func applyCEFSize(_ size: NSSize) {
        applySubviewFrames(browserSize: size)
    }

    private func applySubviewFrames(browserSize: NSSize?) {
        let chrome = Self.viewportChromeInset
        let width = browserSize?.width ?? bounds.width
        let height = browserSize?.height ?? bounds.height
        let frame =
            chrome > 0
            ? NSRect(x: 0, y: 0, width: width, height: height)
            : NSRect(origin: .zero, size: NSSize(width: bounds.width, height: bounds.height))
        for subview in subviews {
            if subview.frame != frame {
                subview.frame = frame
            }
        }
    }
}
