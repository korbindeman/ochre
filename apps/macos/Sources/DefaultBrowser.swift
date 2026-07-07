import AppKit

/// macOS default-browser integration — register as an http/https handler, check
/// whether this build is already the system default, and request the handoff.
enum DefaultBrowser {
    /// True when Launch Services would route both `http` and `https` to this app.
    static var isDefault: Bool {
        isDefault(for: "http") && isDefault(for: "https")
    }

    /// Ask Launch Services to make this app the default http/https handler, then
    /// verify the handoff stuck. Returns false when the OS rejects the request
    /// (common for unsigned dev builds).
    @discardableResult
    static func setAsDefault() -> Bool {
        let appURL = Bundle.main.bundleURL
        let workspace = NSWorkspace.shared
        workspace.setDefaultApplication(at: appURL, toOpenURLsWithScheme: "http")
        workspace.setDefaultApplication(at: appURL, toOpenURLsWithScheme: "https")
        return isDefault
    }

    /// Deep-link into System Settings → Desktop & Dock → Default web browser.
    static func openSystemSettings() {
        guard let url = URL(string: "x-apple.systempreferences:com.apple.Desktop-Settings.extension?DefaultAppWebBrowser") else {
            return
        }
        NSWorkspace.shared.open(url)
    }

    private static func isDefault(for scheme: String) -> Bool {
        guard let probe = URL(string: "\(scheme)://example.com/"),
              let appURL = NSWorkspace.shared.urlForApplication(toOpen: probe) else {
            return false
        }
        return appURL == Bundle.main.bundleURL
    }
}
