import AppKit
import Foundation
import SwiftUI

/// Auto-update state (ADR-0052, Zinnia model).
///
/// Core owns check / download / minisign verify / stage + relaunch helper.
/// This is the host projection: sidebar card + menu action + startup check.
@MainActor
final class AppUpdateModel: ObservableObject {
    static let shared = AppUpdateModel()

    /// Shown in the sidebar when a newer version is available.
    @Published var available: AvailableUpdate?
    /// True while download+install is running.
    @Published var isInstalling = false
    /// 0…1 while installing when total size is known; nil when indeterminate.
    @Published var progress: Double?
    /// Transient error under the card (cleared on dismiss / retry).
    @Published var errorMessage: String?
    /// Just-updated banner after a successful relaunch (`Updated to vX`).
    @Published var justUpdatedVersion: String?

    private var checkTask: Task<Void, Never>?
    private var progressTimer: Timer?
    private static let lastVersionKey = BrowserProfile.defaultsKey("lastAppVersion")
    private static let checkInterval: TimeInterval = 2 * 60 * 60

    struct AvailableUpdate: Equatable {
        var version: String
        var currentVersion: String
        var notes: String
        var json: String
    }

    /// Marketing version of the running build (Info.plist).
    var currentVersion: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.0.0"
    }

    /// Dev bundle ids skip auto-check (noisy on every debug launch). Menu still works.
    var autoCheckEnabled: Bool {
        let id = Bundle.main.bundleIdentifier ?? ""
        return !id.hasSuffix(".dev")
    }

    private var appBundlePath: String {
        Bundle.main.bundleURL.path
    }

    /// Call once after the window is up: show "Updated to vX" if needed, then check.
    func start() {
        noteLaunchVersion()
        // Harness: force the sidebar card without a network round-trip so it can
        // be inspected/screenshotted (mirrors BROWSER_VAULT_DEMO / HISTORY_OPEN).
        if ProcessInfo.processInfo.environment["BROWSER_UPDATE_DEMO"] != nil {
            showDemoUpdate()
            return
        }
        guard autoCheckEnabled else { return }
        scheduleChecks()
    }

    /// Seed a fake available update for UI review (no install path).
    func showDemoUpdate() {
        let version = "9.9.9-demo"
        let current = currentVersion
        let json =
            "{\"version\":\"\(version)\",\"current_version\":\"\(current)\",\"notes\":\"Demo update card.\",\"url\":\"https://github.com/korbindeman/ochre/releases/download/v9.9.9/\(Product.name).app.tar.gz\",\"signature\":\"\"}"
        available = AvailableUpdate(
            version: version, currentVersion: current, notes: "Demo update card.", json: json)
        errorMessage = nil
        isInstalling = false
        progress = nil
    }

    func scheduleChecks() {
        checkTask?.cancel()
        checkTask = Task { [weak self] in
            await self?.performCheck(manual: false)
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: UInt64(Self.checkInterval * 1_000_000_000))
                if Task.isCancelled { break }
                await self?.performCheck(manual: false)
            }
        }
    }

    /// Menu → Check for Updates…
    func checkForUpdatesManual() {
        Task { await performCheck(manual: true) }
    }

    func dismiss() {
        available = nil
        errorMessage = nil
        browser_update_dismiss()
    }

    func dismissJustUpdated() {
        justUpdatedVersion = nil
    }

    /// Download, verify, stage, then quit so the helper can swap + relaunch.
    func updateAndRestart() {
        guard let update = available, !isInstalling else { return }
        isInstalling = true
        errorMessage = nil
        progress = 0
        startProgressPolling()

        let json = update.json
        let appPath = appBundlePath
        Task.detached(priority: .userInitiated) {
            let ok: Bool = json.withCString { jsonPtr in
                appPath.withCString { pathPtr in
                    browser_update_download_and_install(pathPtr, jsonPtr)
                }
            }
            await MainActor.run {
                self.stopProgressPolling()
                self.isInstalling = false
                if ok {
                    // Helper is waiting for our PID to die - quit cleanly so CEF
                    // tears down, then the script swaps the .app and relaunches.
                    (NSApp.delegate as? AppDelegate)?.quit(nil)
                } else {
                    self.progress = nil
                    self.errorMessage = "Could not download the update. Try again."
                }
            }
        }
    }

    // MARK: - Internals

    private func performCheck(manual: Bool) async {
        let version = currentVersion
        let json: String? = await Task.detached(priority: .utility) {
            version.withCString { ptr in
                guard let raw = browser_update_check(ptr) else { return nil as String? }
                defer { browser_string_free(raw) }
                return String(cString: raw)
            }
        }.value

        if let json, let parsed = Self.parse(json: json) {
            available = parsed
            errorMessage = nil
        } else if manual {
            available = nil
            let alert = NSAlert()
            alert.messageText = "You're up to date"
            alert.informativeText = "\(Product.name) \(currentVersion) is the latest version."
            alert.alertStyle = .informational
            alert.addButton(withTitle: "OK")
            alert.runModal()
        }
    }

    private func noteLaunchVersion() {
        let current = currentVersion
        let previous = UserDefaults.standard.string(forKey: Self.lastVersionKey)
        if let previous, previous != current {
            justUpdatedVersion = current
        }
        UserDefaults.standard.set(current, forKey: Self.lastVersionKey)
    }

    private func startProgressPolling() {
        stopProgressPolling()
        progressTimer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) {
            [weak self] _ in
            MainActor.assumeIsolated {
                var downloaded: UInt64 = 0
                var total: UInt64 = 0
                browser_update_download_progress(&downloaded, &total)
                if total > 0 {
                    self?.progress = min(1.0, Double(downloaded) / Double(total))
                } else if downloaded > 0 {
                    self?.progress = nil
                }
            }
        }
    }

    private func stopProgressPolling() {
        progressTimer?.invalidate()
        progressTimer = nil
    }

    private static func parse(json: String) -> AvailableUpdate? {
        guard let data = json.data(using: .utf8),
            let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let version = obj["version"] as? String,
            let current = obj["current_version"] as? String
        else { return nil }
        let notes = obj["notes"] as? String ?? ""
        return AvailableUpdate(
            version: version, currentVersion: current, notes: notes, json: json)
    }
}

// MARK: - Sidebar card (Arc / Claude / Zinnia toast)

/// Compact card at the bottom of the sidebar when an update is available, or
/// briefly after a successful update ("Updated to vX" + release notes).
struct UpdateSidebarCard: View {
    @ObservedObject private var model = AppUpdateModel.shared

    var body: some View {
        if let version = model.justUpdatedVersion {
            card {
                HStack(alignment: .center, spacing: 8) {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Updated to v\(version)")
                            .font(.system(size: 12, weight: .semibold))
                        Button("Release Notes") {
                            if let url = URL(
                                string: "https://github.com/korbindeman/ochre/releases")
                            {
                                NSWorkspace.shared.open(url)
                            }
                        }
                        .buttonStyle(.plain)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 4)
                    dismissButton { model.dismissJustUpdated() }
                }
            }
        } else if model.available != nil {
            card {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(alignment: .center, spacing: 8) {
                        Text("Update available")
                            .font(.system(size: 12, weight: .semibold))
                        Spacer(minLength: 4)
                        if !model.isInstalling {
                            dismissButton { model.dismiss() }
                        }
                    }

                    if model.isInstalling {
                        if let p = model.progress {
                            ProgressView(value: p)
                                .progressViewStyle(.linear)
                        } else {
                            ProgressView()
                                .controlSize(.small)
                        }
                        Text("Downloading…")
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                    } else {
                        Button {
                            model.updateAndRestart()
                        } label: {
                            HStack(spacing: 6) {
                                Image(systemName: "arrow.down.circle.fill")
                                    .font(.system(size: 12, weight: .semibold))
                                Text("Update & Restart")
                                    .font(.system(size: 12, weight: .semibold))
                            }
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 8)
                            .foregroundStyle(.white)
                            .background(Color.accentColor, in: Capsule())
                        }
                        .buttonStyle(.plain)
                    }

                    if let err = model.errorMessage {
                        Text(err)
                            .font(.system(size: 11))
                            .foregroundStyle(.red)
                    }
                }
            }
        }
    }

    private func card<Content: View>(@ViewBuilder content: () -> Content) -> some View {
        content()
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Color(nsColor: .controlBackgroundColor))
            }
            .overlay {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1)
            }
    }

    private func dismissButton(action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: "xmark")
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 18, height: 18)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Dismiss")
    }
}
