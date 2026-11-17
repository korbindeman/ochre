import AppKit
import UniformTypeIdentifiers

/// Arc `StorableSidebar.json` import - auto-detects Arc on this Mac, previews counts,
/// then merge/replaces the sidebar. Settings is the primary surface; File → Import
/// is a shortcut to the same flow.
enum ArcImportFlow {
    struct Preview: Equatable {
        let spaces: Int
        let folders: Int
        let tabs: Int
        let favorites: Int

        var isEmpty: Bool { spaces == 0 && tabs == 0 && favorites == 0 }

        var summary: String {
            var parts: [String] = []
            parts.append("\(spaces) space\(spaces == 1 ? "" : "s")")
            parts.append("\(folders) folder\(folders == 1 ? "" : "s")")
            parts.append("\(tabs) pinned tab\(tabs == 1 ? "" : "s")")
            if favorites > 0 {
                parts.append("\(favorites) favorite\(favorites == 1 ? "" : "s")")
            }
            return parts.joined(separator: ", ")
        }
    }

    struct ImportCounts: Equatable {
        let spaces: Int
        let folders: Int
        let tabs: Int
        let favorites: Int
        let skippedDuplicates: Int
    }

    private struct ImportError: Error, LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    /// Arc's sidebar file when Arc is (or was) installed on this Mac.
    static func defaultSidebarURL() -> URL? {
        guard let c = browser_import_arc_default_path() else { return nil }
        defer { browser_string_free(c) }
        return URL(fileURLWithPath: String(cString: c))
    }

    /// The sidebar file if it exists on disk - no file picker required.
    static func detectedSidebarURL() -> URL? {
        guard let url = defaultSidebarURL(),
            FileManager.default.isReadableFile(atPath: url.path)
        else { return nil }
        return url
    }

    /// A home-shortened path for display in Settings.
    static func displayPath(for url: URL) -> String {
        let path = url.path
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        if path.hasPrefix(home) {
            return "~" + path.dropFirst(home.count)
        }
        return path
    }

    static func loadPreview(for url: URL) -> Preview? {
        if case .success(let preview) = preview(url) { return preview }
        return nil
    }

    static func loadPreviewError(for url: URL) -> String? {
        if case .failure(let error) = preview(url) { return error.localizedDescription }
        return nil
    }

    @discardableResult
    static func importSidebar(at url: URL, replace: Bool) -> ImportCounts? {
        if case .success(let counts) = importFile(url, replace: replace) { return counts }
        return nil
    }

    static func importSidebarError(at url: URL, replace: Bool) -> String? {
        if case .failure(let error) = importFile(url, replace: replace) {
            return error.localizedDescription
        }
        return nil
    }

    /// File → Import from Arc: auto-detect, then preview + confirm. No file picker
    /// when Arc's sidebar is on disk.
    @MainActor
    static func runFromMenu() {
        if let url = detectedSidebarURL() {
            runImport(from: url)
            return
        }
        let alert = NSAlert()
        alert.messageText = "Arc sidebar not found"
        alert.informativeText = """
            \(Product.name) looks for Arc's sidebar data at
            ~/Library/Application Support/Arc/StorableSidebar.json.

            If Arc is installed, open it once so the file exists. You can also import \
            from a copy of that file, or set up import in Settings.
            """
        alert.alertStyle = .informational
        alert.addButton(withTitle: "Open Settings")
        alert.addButton(withTitle: "Choose File…")
        alert.addButton(withTitle: "Cancel")
        switch alert.runModal() {
        case .alertFirstButtonReturn:
            BrowserState.shared.openSettings()
            SettingsModel.shared.section = .general
            SettingsModel.shared.reloadArcImport()
        case .alertSecondButtonReturn:
            if let url = chooseSidebarFile() {
                runImport(from: url)
            }
        default:
            break
        }
    }

    /// Manual fallback - e.g. importing a backup copy of `StorableSidebar.json`.
    @MainActor
    static func chooseSidebarFile() -> URL? {
        let panel = NSOpenPanel()
        panel.title = "Choose Arc Sidebar File"
        panel.prompt = "Choose"
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [.json]
        panel.message = "Select Arc's StorableSidebar.json (or a backup copy)."
        if let defaultURL = defaultSidebarURL() {
            panel.directoryURL = defaultURL.deletingLastPathComponent()
            panel.nameFieldStringValue = defaultURL.lastPathComponent
        }
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        return url
    }

    @MainActor
    static func runImport(from url: URL) {
        switch preview(url) {
        case .failure(let error):
            showError(error.localizedDescription)
        case .success(let preview):
            guard !preview.isEmpty else {
                showError("Nothing to import in this file.")
                return
            }
            confirmAndImport(url: url, preview: preview)
        }
    }

    @MainActor
    static func confirmAndImport(url: URL, preview: Preview, replace: Bool? = nil) {
        let doReplace: Bool
        if let replace {
            if replace && !confirmReplace() { return }
            doReplace = replace
        } else {
            let alert = NSAlert()
            alert.messageText = "Import from Arc?"
            alert.informativeText = """
                \(preview.summary)

                Add to Sidebar keeps your current spaces and adds Arc's. Replace Sidebar \
                clears everything first.

                Open (“Today”) tabs from Arc are not in this file and won't import.
                """
            alert.alertStyle = .informational
            alert.addButton(withTitle: "Add to Sidebar")
            alert.addButton(withTitle: "Replace Sidebar")
            alert.addButton(withTitle: "Cancel")
            let choice = alert.runModal()
            if choice == .alertThirdButtonReturn { return }
            let wantsReplace = choice == .alertSecondButtonReturn
            if wantsReplace && !confirmReplace() { return }
            doReplace = wantsReplace
        }

        switch importFile(url, replace: doReplace) {
        case .failure(let error):
            showError(error.localizedDescription)
        case .success(let result):
            showSuccess(result)
        }
    }

    @MainActor
    static func confirmReplace() -> Bool {
        let warn = NSAlert()
        warn.messageText = "Replace your sidebar?"
        warn.informativeText =
            "This closes every tab and removes all spaces, folders, and saved tabs before importing from Arc."
        warn.alertStyle = .warning
        warn.addButton(withTitle: "Replace")
        warn.addButton(withTitle: "Cancel")
        return warn.runModal() == .alertFirstButtonReturn
    }

    @MainActor
    private static func showSuccess(_ result: ImportCounts) {
        let alert = NSAlert()
        alert.messageText = "Import complete"
        var lines = [
            "Imported \(result.spaces) space\(result.spaces == 1 ? "" : "s"), \(result.folders) folder\(result.folders == 1 ? "" : "s"), \(result.tabs) tab\(result.tabs == 1 ? "" : "s")."
        ]
        if result.favorites > 0 {
            lines.append("Arc favorites were added to the first imported space's pinned strip.")
        }
        if result.skippedDuplicates > 0 {
            lines.append(
                "Skipped \(result.skippedDuplicates) duplicate URL\(result.skippedDuplicates == 1 ? "" : "s") already in a space."
            )
        }
        alert.informativeText = lines.joined(separator: " ")
        alert.alertStyle = .informational
        alert.runModal()
    }

    private static func preview(_ url: URL) -> Result<Preview, ImportError> {
        let path = url.path
        return path.withCString { cPath in
            guard let raw = browser_import_arc_preview(cPath) else {
                return .failure(ImportError(message: "Could not read \(url.lastPathComponent)."))
            }
            defer { browser_string_free(raw) }
            return decodeEnvelope(String(cString: raw)).map { payload in
                Preview(
                    spaces: payload["spaces"] as? Int ?? 0,
                    folders: payload["folders"] as? Int ?? 0,
                    tabs: payload["tabs"] as? Int ?? 0,
                    favorites: payload["favorites"] as? Int ?? 0)
            }
        }
    }

    private static func importFile(_ url: URL, replace: Bool) -> Result<ImportCounts, ImportError> {
        let path = url.path
        return path.withCString { cPath in
            guard let raw = browser_import_arc(cPath, replace) else {
                return .failure(ImportError(message: "Import failed."))
            }
            defer { browser_string_free(raw) }
            return decodeEnvelope(String(cString: raw)).map { payload in
                ImportCounts(
                    spaces: payload["spaces"] as? Int ?? 0,
                    folders: payload["folders"] as? Int ?? 0,
                    tabs: payload["tabs"] as? Int ?? 0,
                    favorites: payload["favorites"] as? Int ?? 0,
                    skippedDuplicates: payload["skipped_duplicates"] as? Int ?? 0)
            }
        }
    }

    private static func decodeEnvelope(_ json: String) -> Result<[String: Any], ImportError> {
        guard let data = json.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return .failure(ImportError(message: "Invalid response from import."))
        }
        if object["ok"] as? Bool == true, let payload = object["payload"] as? [String: Any] {
            return .success(payload)
        }
        return .failure(ImportError(message: object["error"] as? String ?? "Import failed."))
    }

    @MainActor
    private static func showError(_ message: String) {
        let alert = NSAlert()
        alert.messageText = "Arc import failed"
        alert.informativeText = message
        alert.alertStyle = .warning
        alert.runModal()
    }
}
