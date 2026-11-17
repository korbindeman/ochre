import AppKit

/// First-run prompt for password access in private windows (defaults off).
@MainActor
enum PrivateWindowPrompt {
    static func showIfNeeded() {
        guard !browser_settings_private_vault_prompt_seen() else { return }
        browser_settings_set_private_vault_prompt_seen(true)

        let alert = NSAlert()
        alert.messageText = "Use Saved Passwords in Private Windows?"
        alert.informativeText =
            "Private windows don't save browsing history or cookies. " +
            "Saved passwords stay off by default. You can enable fill for this session's private windows."
        alert.alertStyle = .informational
        alert.addButton(withTitle: "Not Now")
        alert.addButton(withTitle: "Allow Passwords")
        let choice = alert.runModal()
        if choice == .alertSecondButtonReturn {
            browser_settings_set_private_vault_enabled(true)
        }
    }
}
