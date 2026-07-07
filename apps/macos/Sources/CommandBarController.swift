import AppKit
import Combine
import SwiftUI

/// The ⌘T command bar — a Spotlight-style floating panel over the browser
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
    /// normal use — the resign-key dismissal is the correct product behaviour.
    var keepPresentedForDemo = false

    /// The autocomplete engine, shared across presentations so the controller
    /// can observe it to resize the panel. Local providers (typed-URL row, then
    /// history) run instantly on every keystroke and drive inline autocomplete;
    /// live search suggestions (Google default) are the debounced remote tier,
    /// merged in below.
    let suggestions = SuggestionController(
        local: [URLProvider(), HistoryProvider()],
        remote: [SearchSuggestProvider()]
    )
    /// The SwiftUI hosting view, retained to measure its fitting height.
    var hosting: NSView?
    var suggestionsObserver: AnyCancellable?

    /// Distance from the window's top edge to the command bar's top edge. The
    /// bar's top stays pinned here; it grows *downward* as suggestions appear.
    let barTopOffset: CGFloat = 150
    let barWidth: CGFloat = 620
    let fieldHeight: CGFloat = 60

    var isVisible: Bool { panel?.isVisible ?? false }

    func toggle() {
        isVisible ? hide() : show()
    }

    func show() {
        guard let parent = parentWindow else { return }
        let panel = self.panel ?? makePanel()
        self.panel = panel

        // Fresh state each presentation. The command bar is the ⌘T new-tab
        // surface (editing the current tab's URL is inline in the sidebar now —
        // ADR-0014), so commits always open a new tab and the field starts empty.
        suggestions.reset()
        Navigation.route = .newTab

        // Host the SwiftUI content pinned top/left/right at a fixed width, with
        // its height left to the content — that is what lets `fittingSize`
        // report the height to grow the panel to.
        let hosting = NSHostingView(rootView: CommandBarView(
            controller: suggestions,
            onChoose: { [weak self] suggestion in
                suggestion.run()
                self?.hide()
            }
        ))
        hosting.translatesAutoresizingMaskIntoConstraints = false
        hosting.sizingOptions = [.intrinsicContentSize]
        let container = NSView()
        container.addSubview(hosting)
        NSLayoutConstraint.activate([
            hosting.topAnchor.constraint(equalTo: container.topAnchor),
            hosting.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            hosting.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            hosting.widthAnchor.constraint(equalToConstant: barWidth),
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

        // Hide the panel *before* it is ever placed on screen, then fade it in.
        // A freshly-made panel defaults to alphaValue 1, so ordering it in first
        // (as `addChildWindow` does) and lowering the alpha afterwards let its
        // not-yet-composited-as-glass content present one opaque frame — the
        // square that flashed under the bar on the *first* ⌘T only. Reused panels
        // were already at alpha 0 from the previous fade-out, which is exactly why
        // it never recurred; setting alpha 0 up front makes every presentation
        // behave like a reused one. The content adds its own spring scale-in.
        panel.alphaValue = 0
        positionPanel(height: fieldHeight)
        parent.addChildWindow(panel, ordered: .above)
        panel.makeKeyAndOrderFront(nil)
        NSLog("[focus] commandbar opened → panel is key")
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.16
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            panel.animator().alphaValue = 1
        }
    }

    func hide() {
        guard let panel, panel.isVisible, !isHiding else { return }
        isHiding = true
        // Back to the default target for the next ⌘T.
        Navigation.route = .newTab
        removeKeyMonitor()
        suggestionsObserver = nil
        suggestions.reset()
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = 0.12
            context.timingFunction = CAMediaTimingFunction(name: .easeIn)
            panel.animator().alphaValue = 0
        }, completionHandler: { [weak self] in
            MainActor.assumeIsolated {
                guard let self, let panel = self.panel else { return }
                panel.parent?.removeChildWindow(panel)
                panel.orderOut(nil)
                self.isHiding = false
                // Make the browser window key again, then deterministically
                // return keyboard focus to the active tab's web view. Relying on
                // AppKit to restore the first responder is not enough: if the
                // pre-open responder was the bare window (e.g. after a sidebar
                // click resigned the page), `makeKey()` alone leaves keystrokes
                // falling on the floor. Driving CEF `set_focus` mirrors the
                // tab-switch path and matches Safari/Arc (dismissing the address
                // bar returns focus to the page).
                self.parentWindow?.makeKey()
                browser_focus_active_tab()
                // P0-1 observability: log what AppKit settled on; the paired
                // `[focus] cef on_got_focus` in the Rust log confirms the page
                // actually took keyboard focus.
                let responder = self.parentWindow?.firstResponder
                NSLog(
                    "[focus] commandbar closed → refocus page; window.firstResponder=%@",
                    responder.map { String(describing: type(of: $0)) } ?? "nil"
                )
            }
        })
    }
}
