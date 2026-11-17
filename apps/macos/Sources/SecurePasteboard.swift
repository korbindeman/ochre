import AppKit

/// Clipboard writes for vault secrets. One place so the Passwords page and the
/// quick-access popover can't drift apart on how a password reaches the
/// pasteboard (they did: the popover auto-cleared, the page did not).
///
/// A copied password is both **marked concealed** (so clipboard-history managers
/// and macOS Universal Clipboard skip it) and **auto-cleared** after a delay, so
/// it does not linger where any app polling `NSPasteboard` can read it.
enum SecurePasteboard {
    /// Type recognized by clipboard managers (the nspasteboard.org convention)
    /// to mean "sensitive - do not record". Password managers set it.
    private static let concealedType = NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType")

    /// Copy `secret` to the general pasteboard, marked concealed, and clear it
    /// after `clearAfter` seconds - but only if the user has not copied
    /// something else since (never clobber a later clipboard).
    static func copySecret(_ secret: String, clearAfter seconds: Double = 30) {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(secret, forType: .string)
        pasteboard.setString(secret, forType: concealedType)
        let stamp = pasteboard.changeCount
        Task {
            try? await Task.sleep(for: .seconds(seconds))
            if NSPasteboard.general.changeCount == stamp {
                NSPasteboard.general.clearContents()
            }
        }
    }
}
