import AppKit

extension CommandBarController {
    /// Route arrow/return/escape to the suggestion list with an AppKit key
    /// monitor. A monitor intercepts arrow keys before the focused `TextField`
    /// reliably (unlike SwiftUI `onKeyPress`, which the field can swallow first);
    /// everything else — typing, editing keys — passes straight through.
    func installKeyMonitor() {
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self else { return event }
            // Pull out the Sendable primitives before hopping onto the main
            // actor, so the non-Sendable NSEvent never crosses the boundary.
            let keyCode = event.keyCode
            let modifiers = event.modifierFlags
            let handled = MainActor.assumeIsolated {
                self.handleKey(keyCode: keyCode, modifiers: modifiers)
            }
            return handled ? nil : event
        }
    }

    func removeKeyMonitor() {
        if let keyMonitor {
            NSEvent.removeMonitor(keyMonitor)
            self.keyMonitor = nil
        }
    }

    /// Handle a navigation key. Returns true if consumed (arrows move the
    /// selection, return chooses / submits, escape dismisses); false lets the
    /// event through to the text field for normal editing.
    func handleKey(keyCode: UInt16, modifiers: NSEvent.ModifierFlags) -> Bool {
        guard isVisible,
              modifiers.intersection([.command, .option, .control]).isEmpty
        else {
            return false
        }
        switch keyCode {
        case 126: // up arrow
            suggestions.moveSelection(by: -1)
        case 125: // down arrow
            suggestions.moveSelection(by: 1)
        case 53: // escape
            hide()
        case 36, 76: // return, enter
            if let selected = suggestions.selected() {
                selected.run()
            } else {
                Navigation.open(suggestions.currentQuery)
            }
            hide()
        default:
            return false
        }
        return true
    }
}
