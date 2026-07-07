import AppKit

/// Borderless panels refuse key status by default; the command bar needs it
/// for its text field.
final class KeyablePanel: NSPanel {
    override var canBecomeKey: Bool { true }
}

extension CommandBarController {
    /// Size the panel to the content's fitting height, keeping the top edge
    /// fixed so the field stays put and suggestions extend downward.
    func resizeToFit() {
        guard let hosting else { return }
        positionPanel(height: max(fieldHeight, ceil(hosting.fittingSize.height)))
    }

    func positionPanel(height: CGFloat) {
        guard let parent = parentWindow, let panel else { return }
        let frame = parent.frame
        let topEdge = frame.maxY - barTopOffset
        panel.setFrame(
            NSRect(
                x: frame.midX - barWidth / 2,
                y: topEdge - height,
                width: barWidth,
                height: height
            ),
            display: true
        )
    }

    func makePanel() -> KeyablePanel {
        let panel = KeyablePanel(
            contentRect: NSRect(x: 0, y: 0, width: barWidth, height: fieldHeight),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.isOpaque = false
        panel.backgroundColor = .clear
        // The glass effect draws its own depth; a window shadow would double it.
        panel.hasShadow = false
        panel.isMovable = false
        panel.hidesOnDeactivate = false
        // Dismiss when the user clicks back into the page (the panel loses key).
        NotificationCenter.default.addObserver(
            forName: NSWindow.didResignKeyNotification,
            object: panel,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, !self.keepPresentedForDemo else { return }
                self.hide()
            }
        }
        return panel
    }
}
