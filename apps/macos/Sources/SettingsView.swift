import AppKit
import SwiftUI

/// The Settings page — a native content-area surface with a sidebar of sections
/// (ADR-0010, style §15). Values are core-owned; this view binds over FFI.
struct SettingsView: View {
    @ObservedObject private var model = SettingsModel.shared

    var body: some View {
        VStack(spacing: 0) {
            Toolbar(onClose: { BrowserState.shared.closeSettings() })
            HStack(spacing: 0) {
                SettingsSidebar(selection: $model.section)
                Divider()
                DetailPane(model: model)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .windowBackgroundColor))
        .onExitCommand { BrowserState.shared.closeSettings() }
        .onAppear { model.reload() }
    }
}

// ─── Sections ───────────────────────────────────────────────────────────────

enum SettingsSection: String, CaseIterable, Identifiable {
    case general = "General"
    case appearance = "Appearance"
    case privacy = "Privacy & Security"
    case passwords = "Passwords"

    var id: String { rawValue }

    var icon: String {
        switch self {
        case .general: return "gearshape"
        case .appearance: return "circle.lefthalf.filled"
        case .privacy: return "hand.raised"
        case .passwords: return "key"
        }
    }
}

// ─── Top bar ────────────────────────────────────────────────────────────────

private struct Toolbar: View {
    let onClose: () -> Void

    var body: some View {
        GlassEffectContainer {
            HStack(spacing: 12) {
                Button(action: onClose) {
                    Image(systemName: "xmark")
                        .font(.system(size: 12, weight: .semibold))
                }
                .buttonStyle(.glass)
                .buttonBorderShape(.circle)
                .help("Close")
                Text("Settings")
                    .font(.system(size: 24, weight: .bold))
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 24)
            .padding(.top, 20)
            .padding(.bottom, 14)
        }
    }
}

// ─── Sidebar ──────────────────────────────────────────────────────────────

private struct SettingsSidebar: View {
    @Binding var selection: SettingsSection

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(SettingsSection.allCases) { section in
                SidebarRow(section: section, isSelected: selection == section) {
                    selection = section
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.top, 16)
        .padding(.bottom, 20)
        .frame(width: 200)
    }
}

private struct SidebarRow: View {
    let section: SettingsSection
    let isSelected: Bool
    let action: () -> Void

    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 8) {
                Image(systemName: section.icon)
                    .font(.system(size: 13, weight: .medium))
                    .frame(width: 18)
                Text(section.rawValue)
                    .font(.system(size: 13, weight: isSelected ? .semibold : .regular))
                Spacer(minLength: 0)
            }
            .foregroundStyle(isSelected ? Color.accentColor : Color.primary)
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(
                RoundedRectangle(cornerRadius: 8)
                    .fill(.quaternary)
                    .opacity(isSelected ? 0.85 : (hovering ? 0.55 : 0))
                    .hoverFade(hovering || isSelected)
            )
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}

// ─── Detail panes ─────────────────────────────────────────────────────────

private struct DetailPane: View {
    @ObservedObject var model: SettingsModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                switch model.section {
                case .general: GeneralPane(model: model)
                case .appearance: AppearancePane(model: model)
                case .privacy: PrivacyPane(model: model)
                case .passwords: PasswordsPane(model: model)
                }
            }
            .frame(maxWidth: 520, alignment: .leading)
            .padding(.horizontal, 28)
            .padding(.top, 20)
            .padding(.bottom, 32)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}

// MARK: General

private struct GeneralPane: View {
    @ObservedObject var model: SettingsModel

    var body: some View {
        SettingsHeader(title: "General", subtitle: "Basics for how Browser behaves on this Mac.")
        SettingsGroup("Default Browser") {
            SettingsRow(
                title: model.isDefaultBrowser
                    ? "Browser is the default" : "Browser is not the default",
                subtitle: "Opens http and https links from other apps."
            ) {
                if !model.isDefaultBrowser {
                    Button("Set as Default") { model.setAsDefaultBrowser() }
                        .buttonStyle(.glass)
                } else {
                    Label("Default", systemImage: "checkmark.circle.fill")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(.green)
                }
            }
        }
        SettingsGroup("Search") {
            SettingsControlLabel("Default search engine")
            Picker("Default search engine", selection: $model.searchEngine) {
                ForEach(SearchEngineOption.allCases) { option in
                    Text(option.label).tag(option)
                }
            }
            .labelsHidden()
            .onChange(of: model.searchEngine) { _, value in
                model.setSearchEngine(value)
            }
            SettingsFootnote("Used for searches from the address bar and command bar.")
        }
        ArcImportSettingsGroup(model: model)
        SettingsGroup("About") {
            SettingsRow(
                title: "Version",
                subtitle: "Browser for macOS"
            ) {
                Text(model.appVersion)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
        }
    }
}

// MARK: Arc import

private struct ArcImportSettingsGroup: View {
    @ObservedObject var model: SettingsModel

    var body: some View {
        SettingsGroup("Switching from Arc") {
            if let url = model.arcSidebarURL, let preview = model.arcPreview {
                SettingsRow(
                    title: "Arc sidebar found",
                    subtitle: ArcImportFlow.displayPath(for: url)
                ) {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                }
                Text(preview.summary)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                HStack(spacing: 10) {
                    Button("Add to Sidebar") {
                        model.importArcMerge()
                    }
                    .buttonStyle(.glass)
                    Button("Replace Sidebar") {
                        model.importArcReplace()
                    }
                    .buttonStyle(.glass)
                }
                Button("Choose another file…") {
                    model.chooseArcSidebarFile()
                }
                .buttonStyle(.plain)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
            } else {
                SettingsRow(
                    title: "Arc sidebar not found",
                    subtitle: model.arcNotFoundExplanation
                ) {
                    EmptyView()
                }
                Button("Choose sidebar file…") {
                    model.chooseArcSidebarFile()
                }
                .buttonStyle(.glass)
            }
            if let status = model.arcImportStatus {
                Text(status)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            SettingsFootnote(model.arcImportFootnote)
        }
    }
}

// MARK: Appearance

private struct AppearancePane: View {
    @ObservedObject var model: SettingsModel

    var body: some View {
        SettingsHeader(
            title: "Appearance",
            subtitle: "One theme for native chrome and web content (ADR-0030)."
        )
        SettingsGroup("Theme") {
            SettingsControlLabel("Color scheme")
            Picker("Color scheme", selection: $model.themePreference) {
                Text("System").tag(Int32(0))
                Text("Light").tag(Int32(1))
                Text("Dark").tag(Int32(2))
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .onChange(of: model.themePreference) { _, value in
                model.applyTheme(value)
            }
            SettingsFootnote(
                "System follows macOS. Light and Dark stay fixed regardless of the OS.")
        }
        SettingsGroup("Address Bar") {
            SettingsControlLabel("Show in")
            Picker("Show in", selection: $model.addressBarPlacement) {
                ForEach(AddressBarPlacement.allCases) { placement in
                    Text(placement.label).tag(placement)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .onChange(of: model.addressBarPlacement) { _, value in
                model.setAddressBarPlacement(value)
            }
            SettingsFootnote(
                "Sidebar keeps the URL chip in the tab list. Page floats it over the web content."
            )
        }
        SettingsGroup("Sidebar") {
            SettingsToggle(title: "Match page color", isOn: $model.sidebarPageTintEnabled) {
                model.setSidebarPageTintEnabled($0)
            }
            SettingsFootnote(
                "Tints the sidebar to the active tab's theme color. Updates when you navigate."
            )
        }
    }
}

// MARK: Privacy

private struct PrivacyPane: View {
    @ObservedObject var model: SettingsModel

    var body: some View {
        SettingsHeader(
            title: "Privacy & Security",
            subtitle: "Ad blocking and cookie-consent handling."
        )
        SettingsGroup("Blocking") {
            SettingsToggle(title: "Ads and trackers", isOn: $model.adblockEnabled) {
                model.setAdblockEnabled($0)
            }
            Divider().opacity(0.5)
            SettingsToggle(title: "Cookie notices", isOn: $model.consentEnabled) {
                model.setConsentEnabled($0)
            }
            SettingsFootnote(
                "Ads and trackers are blocked on the network. Cookie notices reject consent prompts and hide leftover banners."
            )
        }
    }
}

// MARK: Passwords

private struct PasswordsPane: View {
    @ObservedObject var model: SettingsModel

    private var app: AppDelegate? { NSApp.delegate as? AppDelegate }

    var body: some View {
        SettingsHeader(
            title: "Passwords",
            subtitle: "Vault security and quick access to saved logins."
        )
        SettingsGroup("Auto-Lock") {
            SettingsControlLabel("Lock vault")
            Picker("Lock vault", selection: $model.vaultAutoLock) {
                ForEach(VaultAutoLockPolicy.allCases) { option in
                    Text(option.label).tag(option)
                }
            }
            .labelsHidden()
            .onChange(of: model.vaultAutoLock) { _, value in
                model.setVaultAutoLock(value)
            }
            SettingsFootnote(model.vaultAutoLock.help)
        }
        SettingsGroup("Unlock") {
            SettingsToggle(title: "Touch ID", isOn: .constant(false)) { _ in }
                .disabled(true)
            SettingsFootnote("Requires a signed build — coming soon.")
        }
        SettingsGroup("Manage") {
            SettingsRow(
                title: "Open Passwords",
                subtitle: model.vaultConfigured
                    ? "View, search, and edit saved logins."
                    : "Set up the vault to save and fill passwords."
            ) {
                Button("Open…") { app?.showVault() }
                    .buttonStyle(.glass)
            }
            if model.vaultConfigured && !model.vaultLocked {
                SettingsRow(
                    title: "Lock Now",
                    subtitle: "Require your master password to access logins again."
                ) {
                    Button("Lock") {
                        browser_vault_lock()
                        model.reload()
                        BrowserState.shared.refreshVaultMatches()
                    }
                    .buttonStyle(.glass)
                }
            }
        }
    }
}

// ─── Shared controls ──────────────────────────────────────────────────────

private struct SettingsHeader: View {
    let title: String
    let subtitle: String

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.system(size: 20, weight: .bold))
            Text(subtitle)
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
        }
        .padding(.bottom, 18)
    }
}

private struct SettingsGroup<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title)
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 12) {
                content
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 10)
                    .fill(.quaternary.opacity(0.35))
            )
        }
        .padding(.bottom, 20)
    }
}

private struct SettingsRow<Trailing: View>: View {
    let title: String
    let subtitle: String
    @ViewBuilder let trailing: Trailing

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.system(size: 13, weight: .medium))
                Text(subtitle)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 12)
            trailing
        }
    }
}

private struct SettingsControlLabel: View {
    let title: String

    init(_ title: String) {
        self.title = title
    }

    var body: some View {
        Text(title)
            .font(.system(size: 13, weight: .medium))
    }
}

private struct SettingsFootnote: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(text)
            .font(.system(size: 11))
            .foregroundStyle(.tertiary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

private struct SettingsToggle: View {
    let title: String
    @Binding var isOn: Bool
    let onChange: (Bool) -> Void

    var body: some View {
        Toggle(
            isOn: Binding(
                get: { isOn },
                set: { newValue in
                    isOn = newValue
                    onChange(newValue)
                }
            )
        ) {
            Text(title)
                .font(.system(size: 13, weight: .medium))
        }
        .toggleStyle(.switch)
    }
}

// ─── Model ────────────────────────────────────────────────────────────────

enum AddressBarPlacement: String, CaseIterable, Identifiable {
    case sidebar
    case page

    var id: String { rawValue }

    var label: String {
        switch self {
        case .sidebar: return "Sidebar"
        case .page: return "Page"
        }
    }
}

enum SearchEngineOption: Int32, CaseIterable, Identifiable {
    case google = 0
    case duckDuckGo = 1
    case bing = 2
    case brave = 3
    case startpage = 4
    case ecosia = 5
    case kagi = 6

    var id: Int32 { rawValue }

    var label: String {
        switch self {
        case .google: return "Google"
        case .duckDuckGo: return "DuckDuckGo"
        case .bing: return "Bing"
        case .brave: return "Brave Search"
        case .startpage: return "Startpage"
        case .ecosia: return "Ecosia"
        case .kagi: return "Kagi"
        }
    }

    static var current: Self {
        from(engine: browser_settings_search_engine())
    }

    static func from(engine: Int32) -> Self {
        allCases.first { $0.rawValue == engine } ?? .google
    }
}

enum VaultAutoLockPolicy: Int32, CaseIterable, Identifiable {
    case never = 0
    case onBrowserClose = 1
    case onMacRestart = 2
    case onPasswordUse = 3

    var id: Int32 { rawValue }

    var label: String {
        switch self {
        case .never: return "Never"
        case .onBrowserClose: return "When Browser closes"
        case .onMacRestart: return "When Mac restarts"
        case .onPasswordUse: return "After each password"
        }
    }

    var help: String {
        switch self {
        case .never:
            return "Stay unlocked until you choose Lock Now."
        case .onBrowserClose:
            return
                "Lock when you quit Browser. Quitting and reopening Browser is the same thing — your vault is always locked on relaunch."
        case .onMacRestart:
            return
                "Lock when your Mac reboots, not when you quit Browser. Reopening Browser still requires unlock — the vault never persists across launches."
        case .onPasswordUse:
            return "Lock immediately after filling or copying a password."
        }
    }

    static func from(policy: Int32) -> Self {
        allCases.first { $0.rawValue == policy } ?? .onBrowserClose
    }
}

@MainActor
final class SettingsModel: ObservableObject {
    static let shared = SettingsModel()

    @Published var section: SettingsSection = .general
    @Published var themePreference: Int32 = 0
    @Published var adblockEnabled = true
    @Published var consentEnabled = true
    @Published var vaultAutoLock: VaultAutoLockPolicy = .onBrowserClose
    @Published var searchEngine: SearchEngineOption = .google
    @Published var vaultConfigured = false
    @Published var vaultLocked = true
    @Published var isDefaultBrowser = false
    @Published var addressBarPlacement: AddressBarPlacement = .sidebar
    @Published var sidebarPageTintEnabled = false
    @Published var arcSidebarURL: URL?
    @Published var arcPreview: ArcImportFlow.Preview?
    @Published var arcImportStatus: String?

    var arcNotFoundExplanation: String {
        if let path = ArcImportFlow.defaultSidebarURL() {
            return
                "No file at \(ArcImportFlow.displayPath(for: path)). Open Arc once, or choose a copy of StorableSidebar.json."
        }
        return "Arc sidebar import is only available on macOS."
    }

    var arcImportFootnote: String {
        """
        Imports spaces, folders, and pinned tabs from Arc's StorableSidebar.json. \
        Open (“Today”) tabs, easels, and split views are not stored in that file. \
        Add to Sidebar keeps your current layout; Replace clears it first.
        """
    }

    var appVersion: String {
        let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.1"
        let build = Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "1"
        return "\(version) (\(build))"
    }

    func reload() {
        themePreference = browser_theme_preference()
        BrowserState.shared.refreshAdblock()
        adblockEnabled = BrowserState.shared.adblockEnabled
        consentEnabled = BrowserState.shared.consentEnabled
        vaultAutoLock = VaultAutoLockPolicy.from(policy: browser_settings_vault_auto_lock())
        searchEngine = SearchEngineOption.from(engine: browser_settings_search_engine())
        vaultConfigured = browser_vault_is_configured()
        vaultLocked = browser_vault_is_locked()
        isDefaultBrowser = DefaultBrowser.isDefault
        addressBarPlacement =
            BrowserState.shared.isViewportURLBarEnabled ? .page : .sidebar
        sidebarPageTintEnabled = BrowserState.shared.isSidebarPageTintEnabled
        reloadArcImport()
    }

    func reloadArcImport() {
        arcImportStatus = nil
        if let url = ArcImportFlow.detectedSidebarURL() {
            arcSidebarURL = url
            arcPreview = ArcImportFlow.loadPreview(for: url)
        } else {
            arcSidebarURL = nil
            arcPreview = nil
        }
    }

    func chooseArcSidebarFile() {
        guard let url = ArcImportFlow.chooseSidebarFile() else { return }
        arcSidebarURL = url
        if let preview = ArcImportFlow.loadPreview(for: url), !preview.isEmpty {
            arcPreview = preview
            arcImportStatus = "Ready to import from chosen file."
        } else {
            arcPreview = nil
            arcImportStatus =
                ArcImportFlow.loadPreviewError(for: url) ?? "Nothing to import in that file."
        }
    }

    func importArcMerge() {
        importArc(replace: false)
    }

    func importArcReplace() {
        guard ArcImportFlow.confirmReplace() else { return }
        importArc(replace: true)
    }

    private func importArc(replace: Bool) {
        guard let url = arcSidebarURL, let preview = arcPreview, !preview.isEmpty else {
            arcImportStatus = "Choose an Arc sidebar file first."
            return
        }
        if let result = ArcImportFlow.importSidebar(at: url, replace: replace) {
            var parts = [
                "Imported \(result.spaces) space\(result.spaces == 1 ? "" : "s"), \(result.tabs) tab\(result.tabs == 1 ? "" : "s")."
            ]
            if result.skippedDuplicates > 0 {
                parts.append(
                    "Skipped \(result.skippedDuplicates) duplicate URL\(result.skippedDuplicates == 1 ? "" : "s")."
                )
            }
            arcImportStatus = parts.joined(separator: " ")
            reloadArcImport()
        } else {
            arcImportStatus =
                ArcImportFlow.importSidebarError(at: url, replace: replace) ?? "Import failed."
        }
    }

    func setSidebarPageTintEnabled(_ enabled: Bool) {
        BrowserState.shared.setSidebarPageTintEnabled(enabled)
        sidebarPageTintEnabled = enabled
    }

    func setAddressBarPlacement(_ placement: AddressBarPlacement) {
        BrowserState.shared.setViewportURLBarEnabled(placement == .page)
        addressBarPlacement = placement
    }

    func applyTheme(_ preference: Int32) {
        browser_theme_set_preference(preference)
        switch preference {
        case 1: NSApp.appearance = NSAppearance(named: .aqua)
        case 2: NSApp.appearance = NSAppearance(named: .darkAqua)
        default: NSApp.appearance = nil
        }
        let isDark = NSApp.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        browser_theme_set_system_dark(isDark)
    }

    func setAdblockEnabled(_ enabled: Bool) {
        browser_adblock_set_enabled(enabled)
        BrowserState.shared.refreshAdblock()
        adblockEnabled = BrowserState.shared.adblockEnabled
    }

    func setConsentEnabled(_ enabled: Bool) {
        browser_consent_set_enabled(enabled)
        BrowserState.shared.refreshAdblock()
        consentEnabled = BrowserState.shared.consentEnabled
    }

    func setVaultAutoLock(_ policy: VaultAutoLockPolicy) {
        browser_settings_set_vault_auto_lock(policy.rawValue)
        vaultAutoLock = policy
    }

    func setSearchEngine(_ engine: SearchEngineOption) {
        browser_settings_set_search_engine(engine.rawValue)
        searchEngine = engine
    }

    func setAsDefaultBrowser() {
        if !DefaultBrowser.setAsDefault() {
            DefaultBrowser.openSystemSettings()
        }
        isDefaultBrowser = DefaultBrowser.isDefault
    }
}
