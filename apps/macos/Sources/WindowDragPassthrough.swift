import AppKit
import SwiftUI

/// Transparent layer that blocks window-background drags while forwarding pointer
/// events to SwiftUI content underneath.
///
/// Arc-style chrome sets `isMovableByWindowBackground` so the sidebar body moves
/// the window. AppKit decides that at `mouseDown` on the hit-tested view's
/// `mouseDownCanMoveWindow` — toggling the window property mid-drag is too late.
/// This overlay returns `false`, then forwards the click/drag sequence to the real
/// target (hide-and-hit-test-through), so tab reorder drags stay in place.
struct WindowDragPassthrough: NSViewRepresentable {
    func makeNSView(context: Context) -> WindowDragPassthroughNSView {
        let view = WindowDragPassthroughNSView()
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.clear.cgColor
        return view
    }

    func updateNSView(_ nsView: WindowDragPassthroughNSView, context: Context) {}
}

final class WindowDragPassthroughNSView: NSView {
    private weak var dragTarget: NSView?

    override var mouseDownCanMoveWindow: Bool { false }

    override func layout() {
        super.layout()
        if let superview, superview.bounds.size != bounds.size {
            frame = superview.bounds
        }
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        bounds.contains(point) ? self : nil
    }

    override func mouseDown(with event: NSEvent) {
        if let target = hitTestThrough(event) {
            dragTarget = target
            target.mouseDown(with: event)
        } else {
            redispatchWhileHidden(event)
        }
    }

    override func mouseDragged(with event: NSEvent) {
        if let dragTarget {
            dragTarget.mouseDragged(with: event)
        } else {
            redispatchWhileHidden(event)
        }
    }

    override func mouseUp(with event: NSEvent) {
        if let dragTarget {
            dragTarget.mouseUp(with: event)
            self.dragTarget = nil
        } else {
            redispatchWhileHidden(event)
        }
    }

    override func rightMouseDown(with event: NSEvent) {
        if let target = hitTestThrough(event) {
            target.rightMouseDown(with: event)
        } else {
            redispatchWhileHidden(event)
        }
    }

    override func rightMouseUp(with event: NSEvent) {
        if let target = hitTestThrough(event) {
            target.rightMouseUp(with: event)
        } else {
            redispatchWhileHidden(event)
        }
    }

    override func scrollWheel(with event: NSEvent) {
        if let target = hitTestThrough(event) {
            target.scrollWheel(with: event)
        } else {
            redispatchWhileHidden(event)
        }
    }

    override func otherMouseDown(with event: NSEvent) {
        if let target = hitTestThrough(event) {
            target.otherMouseDown(with: event)
        } else {
            redispatchWhileHidden(event)
        }
    }

    override func otherMouseUp(with event: NSEvent) {
        if let target = hitTestThrough(event) {
            target.otherMouseUp(with: event)
        } else {
            redispatchWhileHidden(event)
        }
    }

    /// The overlay wins hit-test to block window drags; when AppKit can't find a
    /// child that accepts the click (SwiftUI `onTapGesture` rows are a common
    /// case), hide momentarily and redispatch so the normal gesture path runs.
    private func redispatchWhileHidden(_ event: NSEvent) {
        isHidden = true
        defer { isHidden = false }
        window?.sendEvent(event)
    }

    /// Hide momentarily so AppKit can hit-test the SwiftUI content underneath.
    private func hitTestThrough(_ event: NSEvent) -> NSView? {
        guard let window else { return nil }
        isHidden = true
        defer { isHidden = false }

        let windowPoint = event.locationInWindow

        if let superview {
            let point = superview.convert(windowPoint, from: nil)
            if let target = superview.hitTest(point), !isSelfOrAncestor(target) {
                return target
            }
        }

        guard let contentView = window.contentView else { return nil }
        let point = contentView.convert(windowPoint, from: nil)
        guard let target = contentView.hitTest(point), !isSelfOrAncestor(target) else {
            return nil
        }
        return target
    }

    private func isSelfOrAncestor(_ view: NSView) -> Bool {
        var current: NSView? = view
        while let node = current {
            if node === self { return true }
            current = node.superview
        }
        return false
    }
}

extension View {
    /// Keep tab (and similar) drags in the sidebar from moving the window.
    func blocksWindowDragPassthrough() -> some View {
        overlay {
            WindowDragPassthrough()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}
