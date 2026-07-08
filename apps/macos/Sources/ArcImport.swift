import AppKit
import UniformTypeIdentifiers

/// Arc `StorableSidebar.json` import — preview, confirm, merge/replace (File menu).
enum ArcImportFlow {
    private struct ImportError: Error, LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    struct Preview {
        let spaces: Int
        let folders: Int
        let tabs: Int
        let favorites: Int
    }

    struct ImportCounts {
        let spaces: Int
        let folders: Int
        let tabs: Int
        let favorites: Int
        let skippedDuplicates: Int
    }

    /// Open a file picker (defaulting to Arc's sidebar file) and run the import flow.
    @MainActor
    static func run(from sender: Any?) {
        let panel = NSOpenPanel()
        panel.title = "Import from Arc"
        panel.prompt = "Preview"
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = [.json]
        if let defaultPath = defaultPath() {
            panel.directoryURL = defaultPath.deletingLastPathComponent()
            panel.nameFieldStringValue = defaultPath.lastPathComponent
        }
        guard panel.runModal() == .OK, let url = panel.url else { return }
        runImport(from: url, sender: sender)
    }

    @MainActor
    private static func runImport(from url: URL, sender: Any?) {
        switch preview(url) {
        case .failure(let error):
            showError(error.localizedDescription, sender: sender)
        case .success(let preview):
            guard preview.spaces > 0 || preview.tabs > 0 || preview.favorites > 0 else {
                showError("Nothing to import in this file.", sender: sender)
                return
            }
            confirmAndImport(url: url, preview: preview, sender: sender)
        }
    }

    @MainActor
    private static func confirmAndImport(url: URL, preview: Preview, sender: Any?) {
        let alert = NSAlert()
        alert.messageText = "Import from Arc?"
        alert.informativeText = """
            \(preview.spaces) space\(preview.spaces == 1 ? "" : "s"), \
            \(preview.folders) folder\(preview.folders == 1 ? "" : "s"), \
            \(preview.tabs) pinned tab\(preview.tabs == 1 ? "" : "s")\
            \(preview.favorites > 0 ? ", \(preview.favorites) favorite\(preview.favorites == 1 ? "" : "s")" : "").

            Merge adds to your current sidebar. Replace clears it first.
            Open (“Today”) tabs from Arc are not stored in this file and won’t import.
            """
        alert.alertStyle = .informational
        alert.addButton(withTitle: "Merge")
        alert.addButton(withTitle: "Replace")
        alert.addButton(withTitle: "Cancel")
        let choice = alert.runModal()
        if choice == .alertThirdButtonReturn { return }
        let replace = choice == .alertSecondButtonReturn
        if replace {
            let warn = NSAlert()
            warn.messageText = "Replace your sidebar?"
            warn.informativeText =
                "This closes every tab and removes all spaces, folders, and saved tabs before importing from Arc."
            warn.alertStyle = .warning
            warn.addButton(withTitle: "Replace")
            warn.addButton(withTitle: "Cancel")
            guard warn.runModal() == .alertFirstButtonReturn else { return }
        }
        switch importFile(url, replace: replace) {
        case .failure(let error):
            showError(error.localizedDescription, sender: sender)
        case .success(let result):
            let done = NSAlert()
            done.messageText = "Import complete"
            var lines = [
                "Imported \(result.spaces) space\(result.spaces == 1 ? "" : "s"), \(result.folders) folder\(result.folders == 1 ? "" : "s"), \(result.tabs) tab\(result.tabs == 1 ? "" : "s")."
            ]
            if result.favorites > 0 {
                lines.append(
                    "Arc favorites were added to the first imported space’s pinned strip.")
            }
            if result.skippedDuplicates > 0 {
                lines.append(
                    "Skipped \(result.skippedDuplicates) duplicate URL\(result.skippedDuplicates == 1 ? "" : "s") already in a space."
                )
            }
            done.informativeText = lines.joined(separator: " ")
            done.alertStyle = .informational
            done.runModal()
        }
    }

    private static func defaultPath() -> URL? {
        guard let c = browser_import_arc_default_path() else { return nil }
        defer { browser_string_free(c) }
        return URL(fileURLWithPath: String(cString: c))
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
    private static func showError(_ message: String, sender: Any?) {
        _ = sender
        let alert = NSAlert()
        alert.messageText = "Arc import failed"
        alert.informativeText = message
        alert.alertStyle = .warning
        alert.runModal()
    }
}
