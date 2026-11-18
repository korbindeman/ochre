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
        // Flush any pending SwiftUI layout first: the `$suggestions` change that
        // triggers us can land a runloop hop before the hosting view re-measures
        // its intrinsic height, so a bare read of `fittingSize` sees the *old* row
        // count. The zero-state (⌘T with an empty field) fires a single update
        // with no debounced follow-up to correct a stale measurement, which left
        // the list clipped at the collapsed height.
        hosting.layoutSubtreeIfNeeded()
        positionPanel(height: max(collapsedHeight, ceil(hosting.fittingSize.height)))
    }

    func positionPanel(height: CGFloat) {
        guard let parent = parentWindow, let panel else { return }
        let frame = parent.frame
        // The glass sits inset by `shadowInset` inside the panel, so lift the
        // panel's top edge by that much to keep the glass's own top pinned at
        // `barTopOffset`.
        let topEdge = frame.maxY - barTopOffset + shadowInset
        panel.setFrame(
            NSRect(
                x: frame.midX - panelWidth / 2,
                y: topEdge - height,
                width: panelWidth,
                height: height
            ),
            display: true
        )
    }

    func makePanel() -> KeyablePanel {
        let panel = KeyablePanel(
            contentRect: NSRect(x: 0, y: 0, width: panelWidth, height: collapsedHeight),
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
                // Scoped: a no-op if another surface already replaced the bar.
                OverlayMachine.closeCommandBar()
            }
        }
        return panel
    }
}
