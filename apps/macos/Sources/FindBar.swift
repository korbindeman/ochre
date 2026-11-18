import AppKit
import SwiftUI

/// The ⌘F find-in-page bar - a small glass panel pinned to the top-right of the
/// window, over the web content.
///
/// Like the command bar (`CommandBarController`) it lives in its own borderless
/// `KeyablePanel` child window rather than a SwiftUI overlay: a separate window
/// composites above CEF's GPU-rendered view and can take key focus for its text
/// field without fighting the web view for first responder. Unlike the command
/// bar it is *not* transient - it stays up (Safari/Chrome behaviour) until
/// Escape or the close button, so clicking into the page keeps it open.
///
/// Search runs through the core's find plumbing (`browser_find` /
/// `browser_stop_finding`); the "n of m" counter is driven by the find-result
/// callback registered in `AppDelegate` (`browserFindResultCallback`).
@MainActor
final class FindBarController {
    weak var parentWindow: NSWindow?
    private var panel: KeyablePanel?
    private let model = FindBarModel()
    private var keyMonitor: Any?
    private var resizeObserver: NSObjectProtocol?

    /// The current search term; empty when the field is cleared or the bar is
    /// closed. Drives `findNext`/`findPrevious` and menu-item validation.
    private var term = ""

    private let barWidth: CGFloat = 340
    private let barHeight: CGFloat = 44
    /// Inset from the window's top-right corner.
    private let margin: CGFloat = 16
    /// Clearance below the (hidden) titlebar so the bar doesn't overlap it.
    private let topInset: CGFloat = 12

    var isVisible: Bool { panel?.isVisible ?? false }
    /// Whether Find Next / Find Previous have something to act on.
    var hasActiveSearch: Bool { isVisible && !term.isEmpty }

    func toggle() {
        isVisible ? hide() : show()
    }

    /// Present the bar and focus its field. Idempotent: if already open, just
    /// re-key it so ⌘F focuses the field (matching Safari).
    func show() {
        guard let parent = parentWindow else { return }
        let panel = self.panel ?? makePanel()
        self.panel = panel

        panel.contentView = NSHostingView(rootView: FindBarView(
            model: model,
            onQueryChange: { [weak self] query in self?.search(query) },
            onNext: { [weak self] in self?.findNext() },
            onPrevious: { [weak self] in self?.findPrevious() },
            onClose: { OverlayMachine.closeFind() }
        ))

        positionPanel()
        // The bar follows the window it was opened in (main or little window).
        if panel.parent !== parent {
            panel.parent?.removeChildWindow(panel)
            parent.addChildWindow(panel, ordered: .above)
        }
        installKeyMonitor()
        observeParentResize()
        panel.alphaValue = 0
        panel.makeKeyAndOrderFront(nil)
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.12
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            panel.animator().alphaValue = 1
        }
    }

    func hide() {
        guard let panel, panel.isVisible else { return }
        removeKeyMonitor()
        stopObservingParentResize()
        // Clear the page's find highlight and reset our state.
        browser_stop_finding(true)
        term = ""
        model.matchText = ""
        panel.parent?.removeChildWindow(panel)
        panel.orderOut(nil)
        // Return keyboard focus to the page, mirroring the command bar's dismiss:
        // making the window key isn't enough on its own, so drive CEF focus too.
        parentWindow?.makeKey()
        browser_focus_active_tab()
    }

    // ─── Search ──────────────────────────────────────────────────────────

    /// Run a fresh search for `query`. Empty clears the session.
    private func search(_ query: String) {
        term = query
        guard !query.isEmpty else {
            browser_stop_finding(true)
            model.matchText = ""
            return
        }
        query.withCString { browser_find($0, true, false, false) }
    }

    /// Step to the next / previous match of the current term.
    func findNext() {
        guard !term.isEmpty else { return }
        term.withCString { browser_find($0, true, false, true) }
    }

    func findPrevious() {
        guard !term.isEmpty else { return }
        term.withCString { browser_find($0, false, false, true) }
    }

    /// Update the "n of m" counter from a find-result callback.
    func updateResult(count: Int, active: Int) {
        guard !term.isEmpty else { model.matchText = ""; return }
        model.matchText = count == 0 ? "No results" : "\(active) of \(count)"
    }

    // ─── Panel ───────────────────────────────────────────────────────────

    /// Pin the bar to the window's top-right corner.
    private func positionPanel() {
        guard let parent = parentWindow, let panel else { return }
        let frame = parent.frame
        panel.setFrame(
            NSRect(
                x: frame.maxX - barWidth - margin,
                y: frame.maxY - barHeight - topInset,
                width: barWidth,
                height: barHeight
            ),
            display: true
        )
    }

    private func makePanel() -> KeyablePanel {
        let panel = KeyablePanel(
            contentRect: NSRect(x: 0, y: 0, width: barWidth, height: barHeight),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.isMovable = false
        panel.hidesOnDeactivate = false
        return panel
    }

    /// Keep the bar pinned top-right when the window resizes (child windows only
    /// track the parent's origin on move, so a top-right anchor drifts on resize).
    private func observeParentResize() {
        guard resizeObserver == nil, let parent = parentWindow else { return }
        resizeObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.didResizeNotification,
            object: parent,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.positionPanel() }
        }
    }

    private func stopObservingParentResize() {
        if let resizeObserver {
            NotificationCenter.default.removeObserver(resizeObserver)
            self.resizeObserver = nil
        }
    }

    // ─── Keyboard ────────────────────────────────────────────────────────

    /// Escape (from anywhere) closes the bar; Return / ⇧Return step matches, but
    /// only while the field is focused (the panel is key) so we never hijack
    /// Return typed into the page. Everything else passes through for editing.
    private func installKeyMonitor() {
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self else { return event }
            let keyCode = event.keyCode
            let modifiers = event.modifierFlags
            let handled = MainActor.assumeIsolated {
                self.handleKey(keyCode: keyCode, modifiers: modifiers)
            }
            return handled ? nil : event
        }
    }

    private func removeKeyMonitor() {
        if let keyMonitor {
            NSEvent.removeMonitor(keyMonitor)
            self.keyMonitor = nil
        }
    }

    private func handleKey(keyCode: UInt16, modifiers: NSEvent.ModifierFlags) -> Bool {
        guard isVisible else { return false }
        let plain = modifiers.intersection([.command, .option, .control]).isEmpty
        if keyCode == 53, plain { // escape
            OverlayMachine.closeFind()
            return true
        }
        // Return steps matches only when typing in the field.
        guard panel?.isKeyWindow == true, keyCode == 36 || keyCode == 76 else { return false }
        if modifiers.contains(.shift) { findPrevious() } else { findNext() }
        return true
    }
}

/// Observable state for the find bar - just the "n of m" match text.
@MainActor
final class FindBarModel: ObservableObject {
    @Published var matchText: String = ""
}

/// C-ABI trampoline for find-in-page results. Registered once at launch via
/// `browser_set_find_result_callback`. A non-capturing global, so it coerces to
/// the expected `@convention(c)` pointer. Invoked on the main thread.
func browserFindResultCallback(_ count: Int32, _ activeOrdinal: Int32, _ finalUpdate: Bool) {
    MainActor.assumeIsolated {
        (NSApp.delegate as? AppDelegate)?.findBar.updateResult(
            count: Int(count), active: Int(activeOrdinal))
    }
}

/// The Liquid Glass find field: magnifier, query field, match counter, and
/// previous / next / close controls.
private struct FindBarView: View {
    @ObservedObject var model: FindBarModel
    var onQueryChange: (String) -> Void
    var onNext: () -> Void
    var onPrevious: () -> Void
    var onClose: () -> Void

    @State private var query = ""
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(.secondary)
            TextField("Find", text: $query)
                .textFieldStyle(.plain)
                .font(.system(size: 14))
                .focused($focused)
                .onChange(of: query) { _, newValue in onQueryChange(newValue) }

            if !model.matchText.isEmpty {
                Text(model.matchText)
                    .font(.system(size: 12))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                    .layoutPriority(1)
            }

            Divider().frame(height: 16)

            iconButton("chevron.up", help: "Previous (⇧⏎)", action: onPrevious)
            iconButton("chevron.down", help: "Next (⏎)", action: onNext)
            iconButton("xmark", help: "Done (esc)", action: onClose)
        }
        .padding(.horizontal, 12)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .glassEffect(.regular, in: .capsule)
        .onAppear { focused = true }
    }

    private func iconButton(_ symbol: String, help: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 12, weight: .semibold))
                .frame(width: 20, height: 20)
        }
        .buttonStyle(IconButtonStyle(cornerRadius: 6))
        .help(help)
    }
}
