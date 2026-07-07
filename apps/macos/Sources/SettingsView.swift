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
                title: "Make Browser the default",
                subtitle: model.isDefaultBrowser
                    ? "Browser is your default for web links."
                    : "Open http and https links in Browser."
            ) {
                if !model.isDefaultBrowser {
                    Button("Set as Default") { model.setAsDefaultBrowser() }
                        .buttonStyle(.glass)
                } else {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                }
            }
        }
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

// MARK: Appearance

private struct AppearancePane: View {
    @ObservedObject var model: SettingsModel

    var body: some View {
        SettingsHeader(
            title: "Appearance",
            subtitle: "One theme for native chrome and web content (ADR-0030)."
        )
        SettingsGroup("Color Scheme") {
            Picker("Appearance", selection: $model.themePreference) {
                Text("System").tag(Int32(0))
                Text("Light").tag(Int32(1))
                Text("Dark").tag(Int32(2))
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .onChange(of: model.themePreference) { _, value in
                model.applyTheme(value)
            }
            Text("System tracks macOS. Light and Dark pin the browser regardless of the OS.")
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
                .padding(.top, 4)
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
        SettingsGroup("Adblock") {
            SettingsToggle(
                title: "Block ads and trackers",
                subtitle: "Network-level blocking on by default.",
                isOn: $model.adblockEnabled
            ) {
                model.setAdblockEnabled($0)
            }
        }
        SettingsGroup("Cookie Notices") {
            SettingsToggle(
                title: "Block cookie notices",
                subtitle: "Reject known consent prompts and hide leftover banners.",
                isOn: $model.consentEnabled
            ) {
                model.setConsentEnabled($0)
            }
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
            Picker("Lock vault", selection: $model.vaultAutoLock) {
                ForEach(VaultAutoLockPolicy.allCases) { option in
                    Text(option.label).tag(option)
                }
            }
            .onChange(of: model.vaultAutoLock) { _, value in
                model.setVaultAutoLock(value)
            }
            Text(model.vaultAutoLock.help)
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
                .padding(.top, 4)
        }
        SettingsGroup("Unlock") {
            SettingsToggle(
                title: "Unlock with Touch ID",
                subtitle: "Requires a signed build — coming soon.",
                isOn: .constant(false)
            ) { _ in }
            .disabled(true)
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
            Text(title.uppercased())
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

private struct SettingsToggle: View {
    let title: String
    let subtitle: String
    @Binding var isOn: Bool
    let onChange: (Bool) -> Void

    var body: some View {
        Toggle(isOn: Binding(
            get: { isOn },
            set: { newValue in
                isOn = newValue
                onChange(newValue)
            }
        )) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.system(size: 13, weight: .medium))
                Text(subtitle)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
        }
        .toggleStyle(.switch)
    }
}

// ─── Model ────────────────────────────────────────────────────────────────

enum VaultAutoLockPolicy: Int32, CaseIterable, Identifiable {
    case never = 0
    case onBrowserClose = 1
    case onRestart = 2
    case onPasswordUse = 3

    var id: Int32 { rawValue }

    var label: String {
        switch self {
        case .never: return "Never"
        case .onBrowserClose: return "When Browser closes"
        case .onRestart: return "When Browser restarts"
        case .onPasswordUse: return "After each password"
        }
    }

    var help: String {
        switch self {
        case .never:
            return "Stay unlocked until you choose Lock Now. The vault is still cleared from memory when Browser quits."
        case .onBrowserClose:
            return "Lock when you quit Browser."
        case .onRestart:
            return "Require your master password each time you open Browser. Also locks when you quit."
        case .onPasswordUse:
            return "Lock immediately after filling or copying a password."
        }
    }

    static func from(policy: Int32) -> Self {
        allCases.first { $0.rawValue == policy } ?? .onRestart
    }
}

@MainActor
final class SettingsModel: ObservableObject {
    static let shared = SettingsModel()

    @Published var section: SettingsSection = .general
    @Published var themePreference: Int32 = 0
    @Published var adblockEnabled = true
    @Published var consentEnabled = true
    @Published var vaultAutoLock: VaultAutoLockPolicy = .onRestart
    @Published var vaultConfigured = false
    @Published var vaultLocked = true
    @Published var isDefaultBrowser = false

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
        vaultConfigured = browser_vault_is_configured()
        vaultLocked = browser_vault_is_locked()
        isDefaultBrowser = DefaultBrowser.isDefault
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

    func setAsDefaultBrowser() {
        if !DefaultBrowser.setAsDefault() {
            DefaultBrowser.openSystemSettings()
        }
        isDefaultBrowser = DefaultBrowser.isDefault
    }
}
