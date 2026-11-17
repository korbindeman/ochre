import AppKit
import SwiftUI

/// Explicit window-drag surface for chrome that must not compete with tab reorder.
///
/// Arc-style chrome used `isMovableByWindowBackground`, but AppKit decides that at
/// `mouseDown` on the hit view's `mouseDownCanMoveWindow` - toggling the window
/// property in an event monitor, or a full-bleed SwiftUI overlay on a gesture tree,
/// does not reliably keep tab drags in place. The sidebar tab list instead disables
/// background drag globally and hosts explicit handles in the top chrome (nav row,
/// URL-bar padding) and the empty scroll viewport below the tab list (Arc).
struct WindowDragHandle: NSViewRepresentable {
    func makeNSView(context: Context) -> WindowDragHandleView {
        WindowDragHandleView()
    }

    func updateNSView(_ nsView: WindowDragHandleView, context: Context) {}
}

final class WindowDragHandleView: NSView {
    override var mouseDownCanMoveWindow: Bool { false }

    override func layout() {
        super.layout()
        if let superview, superview.bounds.size != bounds.size {
            frame = superview.bounds
        }
    }

    override func mouseDown(with event: NSEvent) {
        window?.performDrag(with: event)
    }
}

extension View {
    /// Top-chrome background that drags the window without blocking controls drawn
    /// above it in the same stack.
    func windowDragHandleBackground() -> some View {
        background { WindowDragHandle() }
    }
}

/// Fills the sidebar scroll viewport below the last tab row. Drags the window on
/// press-and-move; a click without movement runs `onTap` (dismiss URL editor, etc.).
struct SidebarEmptyDragArea: NSViewRepresentable {
    var onTap: () -> Void

    func makeNSView(context: Context) -> SidebarEmptyDragAreaView {
        let view = SidebarEmptyDragAreaView()
        view.onTap = onTap
        return view
    }

    func updateNSView(_ nsView: SidebarEmptyDragAreaView, context: Context) {
        nsView.onTap = onTap
    }
}

final class SidebarEmptyDragAreaView: NSView {
    var onTap: (() -> Void)?
    private var mouseDownEvent: NSEvent?
    private var didDrag = false

    /// Enough movement to count as a window drag, not a click.
    private let dragThreshold: CGFloat = 4

    override var mouseDownCanMoveWindow: Bool { false }

    override func layout() {
        super.layout()
        if let superview, superview.bounds.size != bounds.size {
            frame = superview.bounds
        }
    }

    override func mouseDown(with event: NSEvent) {
        mouseDownEvent = event
        didDrag = false
    }

    override func mouseDragged(with event: NSEvent) {
        guard let mouseDownEvent, !didDrag else { return }
        let dx = event.locationInWindow.x - mouseDownEvent.locationInWindow.x
        let dy = event.locationInWindow.y - mouseDownEvent.locationInWindow.y
        guard hypot(dx, dy) >= dragThreshold else { return }
        didDrag = true
        window?.performDrag(with: mouseDownEvent)
    }

    override func mouseUp(with event: NSEvent) {
        defer {
            mouseDownEvent = nil
            didDrag = false
        }
        guard !didDrag else { return }
        onTap?()
    }
}
