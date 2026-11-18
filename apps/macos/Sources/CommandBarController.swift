import AppKit
import Combine
import SwiftUI

/// The ⌘T command bar - a Spotlight-style floating panel over the browser
/// window for web search / URL entry, with favicon autocomplete.
///
/// It lives in its own borderless `NSPanel` (a child window of the browser
/// window) rather than a SwiftUI overlay for two reasons: a separate window is
/// guaranteed to composite above CEF's GPU-rendered view, and becoming the key
/// window gives its text field keyboard focus without fighting the web view
/// for first responder.
///
/// Autocomplete is provided by the reusable `SuggestionController` /
/// `AutocompleteList` component; the command bar is just its first host.
@MainActor
final class CommandBarController {
    weak var parentWindow: NSWindow?
    var panel: KeyablePanel?
    var isHiding = false
    var keyMonitor: Any?
    /// Demo/verification only: keep the bar presented even when its panel resigns
    /// key, so it can be screenshotted without staying frontmost. Never set in
    /// normal use - the resign-key dismissal is the correct product behaviour.
    var keepPresentedForDemo = false

    /// Drives the `glassEffectID` morph in `CommandBarView` so dismiss can shrink
    /// the glass before the panel is torn down.
    let presentation = CommandBarPresentation()

    /// The autocomplete engine, shared across presentations so the controller
    /// can observe it to resize the panel. Local providers (typed URL, open tabs,
    /// history, top sites, search fallback) run instantly; live search suggestions
    /// are the debounced remote tier, merged above the fallback row.
    let suggestions = SuggestionController(
        local: CommandBarProviders.local,
        remote: CommandBarProviders.remote
    )
    /// The SwiftUI hosting view, retained to measure its fitting height.
    var hosting: NSView?
    var suggestionsObserver: AnyCancellable?

    /// Distance from the window's top edge to the command bar's top edge. The
    /// bar's top stays pinned here; it grows *downward* as suggestions appear.
    let barTopOffset: CGFloat = 150
    /// Visible width of the glass surface.
    let barWidth: CGFloat = 620
    /// Transparent margin the panel leaves around the glass on every side, so the
    /// material's own soft drop shadow renders in that margin instead of being
    /// hard-clipped at the panel's edge. The panel is this much larger than the
    /// glass on each side; the SwiftUI content pads itself by the same amount.
    let shadowInset: CGFloat = 36
    /// Height of the field row (the panel's collapsed content height).
    let fieldHeight: CGFloat = 60

    /// Full panel width: the glass plus its shadow margin on both sides.
    var panelWidth: CGFloat { barWidth + shadowInset * 2 }
    /// Panel height with only the field showing (field row + shadow margins).
    var collapsedHeight: CGFloat { fieldHeight + shadowInset * 2 }

    /// Matches `CommandBarView`'s fade, so the panel leaves as the glass does.
    private var fadeOutDuration: TimeInterval {
        MotionPolicy.reduceMotion ? 0.06 : 0.14
    }
    /// Bumped on every show; a pending hide for an older presentation is dropped.
    private var presentationGeneration = 0

    var isVisible: Bool { panel?.isVisible ?? false }

    /// Render the bar as shown. Only `OverlayMachine`'s renderer calls this;
    /// intents go through the machine. `initialQuery` prefills the field
    /// (selected, address-bar style) for editing a URL.
    func show(initialQuery: String = "") {
        guard let parent = parentWindow else { return }
        let panel = self.panel ?? makePanel()
        self.panel = panel

        // Fresh state each presentation. The command bar is the ⌘T new-tab
        // surface (editing the current tab's URL is inline in the sidebar now -
        // ADR-0014), so commits always open a new tab and the field starts empty.
        presentationGeneration += 1
        isHiding = false
        suggestions.reset()
        suggestions.showZeroState()
        presentation.isPresented = false

        // Host the SwiftUI content pinned top/left/right at a fixed width, with
        // its height left to the content - that is what lets `fittingSize`
        // report the height to grow the panel to.
        let hosting = NSHostingView(
            rootView: CommandBarView(
                presentation: presentation,
                controller: suggestions,
                onChoose: { suggestion in
                    suggestion.run()
                    OverlayMachine.closeCommandBar()
                },
                shadowInset: shadowInset,
                barWidth: barWidth,
                initialQuery: initialQuery
            ))
        hosting.translatesAutoresizingMaskIntoConstraints = false
        hosting.sizingOptions = [.intrinsicContentSize]
        let container = NSView()
        container.addSubview(hosting)
        NSLayoutConstraint.activate([
            hosting.topAnchor.constraint(equalTo: container.topAnchor),
            hosting.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            hosting.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            hosting.widthAnchor.constraint(equalToConstant: panelWidth),
        ])
        panel.contentView = container
        self.hosting = hosting

        // Resize the panel to fit whenever the result set changes (next runloop
        // tick, so SwiftUI has laid the new content out first).
        suggestionsObserver = suggestions.$suggestions
            .receive(on: RunLoop.main)
            .sink { [weak self] _ in
                MainActor.assumeIsolated { self?.resizeToFit() }
            }
        installKeyMonitor()

        // The content starts at opacity 0 and fades in (CommandBarView), so the
        // panel itself stays fully opaque: no window-alpha ramp for the glass to
        // ignore, and no raw frame before the glass composites.
        panel.alphaValue = 1
        positionPanel(height: collapsedHeight)
        resizeToFit()
        if panel.parent !== parent {
            panel.parent?.removeChildWindow(panel)
            parent.addChildWindow(panel, ordered: .above)
        }
        panel.makeKeyAndOrderFront(nil)
        NSLog("[focus] commandbar opened → panel is key")
    }

    /// Render the bar as hidden (the machine's renderer only): fade the
    /// content, then take the panel off screen. A reopen during the fade bumps
    /// `presentationGeneration`, which cancels the pending teardown.
    func hide() {
        guard let panel, panel.isVisible, !isHiding else { return }
        isHiding = true
        removeKeyMonitor()
        suggestionsObserver = nil
        presentation.isPresented = false
        let generation = presentationGeneration
        DispatchQueue.main.asyncAfter(deadline: .now() + fadeOutDuration) { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.presentationGeneration == generation else { return }
                self.finishHide(panel: panel)
            }
        }
    }

    private func finishHide(panel: KeyablePanel) {
        isHiding = false
        suggestions.reset()
        panel.parent?.removeChildWindow(panel)
        panel.orderOut(nil)
        // Make the browser window key again, then deterministically return
        // keyboard focus to the active tab's web view. `makeKey()` alone leaves
        // keystrokes falling on the floor when the pre-open responder was the
        // bare window. Only when nothing replaced the bar: an address edit that
        // opened meanwhile owns the keyboard.
        guard OverlayMachine.state == .none else { return }
        parentWindow?.makeKey()
        browser_focus_active_tab()
        let responder = parentWindow?.firstResponder
        NSLog(
            "[focus] commandbar closed → refocus page; window.firstResponder=%@",
            responder.map { String(describing: type(of: $0)) } ?? "nil"
        )
    }
}
