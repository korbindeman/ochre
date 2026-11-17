import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The Passwords page - the native vault surface (ADR-0010/0016/0017, style §15).
///
/// Like History, it is a *content-area surface* hosted by `RootView` in an
/// `NSHostingView` over the web content, not a tab. It shows the unlock screen
/// when the vault is locked and the login list (with on-gesture password reveal)
/// when unlocked. All data comes from the core over the `browser_vault_*` FFI -
/// **passwords cross only via `browser_vault_password`, on an explicit reveal.**
struct VaultView: View {
    @ObservedObject private var model = VaultModel.shared

    var body: some View {
        Group {
            if !model.isLocked {
                VaultList(model: model)
            } else if model.isConfigured {
                UnlockView(model: model)
            } else {
                SetupView(model: model)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .windowBackgroundColor))
        .onExitCommand { BrowserState.shared.closeVault() }
    }
}

// ─── Setup (first-run) ────────────────────────────────────────────────────────

/// First-run setup - log into an existing Bitwarden / Vaultwarden account. On
/// success the vault unlocks and its config is persisted so later launches auto-
/// init; import (which merges) is then a click away in the list.
private struct SetupView: View {
    @ObservedObject var model: VaultModel
    /// Self-hosted (Vaultwarden) base URL. Empty ⇒ the default Bitwarden cloud;
    /// only shown/used when `selfHosted` is on.
    @State private var server = ""
    @State private var email = ""
    @State private var password = ""
    @State private var code = ""
    /// Whether the user is pointing at a self-hosted server instead of the cloud.
    @State private var selfHosted = false

    /// The pending two-factor challenge kind (from the last attempt), or nil.
    private var twoFactor: String? { model.twoFactor }

    /// The server to log into - empty for cloud, the entered URL when self-hosted.
    private var resolvedServer: String { selfHosted ? server : "" }

    private var canSubmit: Bool {
        guard !email.isEmpty, !password.isEmpty, !model.busy else { return false }
        if selfHosted && server.isEmpty { return false }
        // Once a factor is challenged, the code is required to proceed.
        return twoFactor == nil || !code.isEmpty
    }

    /// Copy under the title - adapts to a live two-factor challenge.
    private var subtitle: String {
        switch twoFactor {
        case "email": return "Enter the code we emailed to \(email)."
        case "authenticator", "yubikey": return "Enter the code from your authenticator app."
        case .some: return "Enter your two-factor code to continue."
        case .none where selfHosted: return "Log in to your self-hosted Vaultwarden or Bitwarden."
        default: return "Log in to your Bitwarden account."
        }
    }

    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: "key.horizontal.fill")
                .font(.system(size: 34, weight: .light))
                .foregroundStyle(.tertiary)
            Text("Set Up Passwords")
                .font(.system(size: 18, weight: .semibold))
            Text(subtitle)
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)

            GlassEffectContainer {
                VStack(spacing: 8) {
                    if selfHosted {
                        field("Self-hosted server URL (https://…)", text: $server)
                    }
                    field("Email", text: $email)
                    secureField("Master password", text: $password)
                    field(
                        twoFactor == nil ? "Two-factor code (if enabled)" : "Two-factor code",
                        text: $code)
                }
            }
            .frame(maxWidth: 320)

            if let error = model.actionError {
                Text(error)
                    .font(.system(size: 11))
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 320)
            }

            Button {
                model.setup(
                    server: resolvedServer, email: email, password: password,
                    code: code, provider: twoFactor ?? "")
            } label: {
                if model.busy {
                    ProgressView().controlSize(.small)
                } else {
                    Text(twoFactor == nil ? "Log In" : "Verify").frame(maxWidth: 120)
                }
            }
            .buttonStyle(.glass)
            .disabled(!canSubmit)

            // Email codes can be re-sent (the others live on the user's device).
            if twoFactor == "email" {
                Button("Resend code") {
                    model.resendEmailCode(email: email, password: password)
                }
                .buttonStyle(.plain)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .disabled(model.busy)
            }

            // Cloud is the default; self-hosting is one tap away. Hidden mid-2FA,
            // where the server is already settled.
            if twoFactor == nil {
                Button(selfHosted ? "Use Bitwarden.com instead" : "Use a self-hosted server") {
                    selfHosted.toggle()
                    if !selfHosted { server = "" }
                    model.actionError = nil
                }
                .buttonStyle(.plain)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .disabled(model.busy)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(40)
    }

    private func field(_ prompt: String, text: Binding<String>) -> some View {
        TextField(prompt, text: text)
            .textFieldStyle(.plain)
            .font(.system(size: 13))
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .glassEffect(.regular, in: .capsule)
    }

    private func secureField(_ prompt: String, text: Binding<String>) -> some View {
        SecureField(prompt, text: text)
            .textFieldStyle(.plain)
            .font(.system(size: 13))
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .glassEffect(.regular, in: .capsule)
    }
}

// ─── Unlock ─────────────────────────────────────────────────────────────────

/// Master-password unlock. v0 unlocks offline from the cache; the full
/// account-setup + online-login flow (server URL, email, 2FA) is a follow-up.
private struct UnlockView: View {
    @ObservedObject var model: VaultModel
    @ObservedObject private var state = BrowserState.shared
    @State private var password = ""
    @FocusState private var passwordFocused: Bool

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "lock.fill")
                .font(.system(size: 34, weight: .light))
                .foregroundStyle(.tertiary)
            Text("Vault Locked")
                .font(.system(size: 18, weight: .semibold))
            Text("Enter your master password to unlock.")
                .font(.system(size: 12))
                .foregroundStyle(.secondary)

            SecureField("Master password", text: $password)
                .textFieldStyle(.plain)
                .font(.system(size: 13))
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .glassEffect(.regular, in: .capsule)
                .frame(maxWidth: 300)
                .focused($passwordFocused)
                .onSubmit { model.unlock(password: password) }

            if let error = model.actionError {
                Text(error)
                    .font(.system(size: 11))
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.center)
            }

            Button {
                model.unlock(password: password)
            } label: {
                if model.busy {
                    ProgressView().controlSize(.small)
                } else {
                    Text("Unlock").frame(maxWidth: 120)
                }
            }
            .buttonStyle(.glass)
            .disabled(password.isEmpty || model.busy)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(40)
        .onAppear { focusPassword() }
        .onChange(of: state.isVaultOpen) { _, open in
            if open { focusPassword() }
        }
        .onChange(of: model.isLocked) { _, locked in
            if locked { focusPassword() }
        }
    }

    /// Defer one runloop so `RootView`'s `makeFirstResponder(vaultHost)` wins
    /// before SwiftUI assigns field focus (same pattern as FindBar / URL bar).
    private func focusPassword() {
        DispatchQueue.main.async { passwordFocused = true }
    }
}

// ─── Login list ───────────────────────────────────────────────────────────────

private struct VaultList: View {
    @ObservedObject var model: VaultModel

    var body: some View {
        Group {
            if model.filtered.isEmpty {
                EmptyState(query: model.query, model: model)
            } else {
                List {
                    ForEach(model.filtered) { login in
                        VaultRow(login: login, model: model)
                            .listRowInsets(EdgeInsets(top: 0, leading: 16, bottom: 0, trailing: 16))
                            .listRowSeparator(.hidden)
                    }
                }
                .listStyle(.plain)
                .scrollContentBackground(.hidden)
                // Match History's shell (style §247): fade content under the
                // material-free floating toolbar, and give the last row room to
                // clear the window edge (`safeAreaPadding`, since
                // `contentMargins(.scrollContent)` is a no-op on `List`).
                .scrollEdgeEffectStyle(.soft, for: .top)
                .safeAreaPadding(.bottom, 20)
            }
        }
        .safeAreaBar(edge: .top, spacing: 0) { Toolbar(model: model) }
        .sheet(item: $model.editing) { target in
            LoginEditor(model: model, target: target)
        }
    }
}

/// Title + Liquid Glass search + a Lock button - the macOS 26 toolbar idiom that
/// History uses (glass on the capsule controls, native content below).
private struct Toolbar: View {
    @ObservedObject var model: VaultModel

    var body: some View {
        // Group the glass controls in one container so they share a sampling
        // region and blend as a single material (never stack glass on glass).
        GlassEffectContainer {
            HStack(spacing: 12) {
                // Leading dismiss - mirrors History (ADR-0010): a surface is exclusive
                // with the tabs, so ✕ returns to the page (Escape also works).
                Button {
                    BrowserState.shared.closeVault()
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 12, weight: .semibold))
                }
                .buttonStyle(.glass)
                .buttonBorderShape(.circle)
                .help("Close")
                Text("Passwords")
                    .font(.system(size: 24, weight: .bold))
                Spacer(minLength: 16)
                SearchField(text: $model.query)
                    .frame(maxWidth: 280)
                Button {
                    model.editing = EditorTarget(detail: nil)
                } label: {
                    Label("Add", systemImage: "plus")
                }
                .buttonStyle(.glass)
                .fixedSize()
                .disabled(model.busy)
                .help("Add a new login")
                Button {
                    importPasswords(model)
                } label: {
                    if model.busy {
                        ProgressView().controlSize(.small)
                    } else {
                        Label("Import", systemImage: "square.and.arrow.down")
                    }
                }
                .buttonStyle(.glass)
                .fixedSize()
                .disabled(model.busy)
                .help("Import passwords from a CSV or Bitwarden JSON export (merges)")
                Button {
                    model.lock()
                } label: {
                    Label("Lock", systemImage: "lock")
                }
                .buttonStyle(.glass)
                .fixedSize()
            }
            .padding(.horizontal, 24)
            .padding(.top, 20)
            .padding(.bottom, 14)
            .overlay(alignment: .bottomTrailing) {
                if let status = model.importStatus {
                    Text(status)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 24)
                }
            }
        }
    }
}

/// Present an open panel for a CSV / Bitwarden-JSON export and hand it to the
/// model to merge-import.
@MainActor
private func importPasswords(_ model: VaultModel) {
    let panel = NSOpenPanel()
    panel.allowedContentTypes = [.commaSeparatedText, .json, .plainText]
    panel.allowsMultipleSelection = false
    panel.prompt = "Import"
    panel.message = "Choose a CSV or Bitwarden JSON export to merge into your vault."
    if panel.runModal() == .OK, let url = panel.url {
        model.importFile(url)
    }
}

private struct SearchField: View {
    @Binding var text: String

    var body: some View {
        HStack(spacing: 7) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.secondary)
            TextField("Search passwords", text: $text)
                .textFieldStyle(.plain)
            if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
            }
        }
        .font(.system(size: 13))
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .glassEffect(.regular, in: .capsule)
    }
}

/// One login row: favicon · name / username · reveal + copy affordances. The
/// password is fetched from the core only when revealed (never listed).
private struct VaultRow: View {
    let login: VaultLogin
    @ObservedObject var model: VaultModel

    @State private var hovering = false
    @State private var revealed: String?
    /// Flashes a checkmark on the copy button so the copy visibly lands (§17,
    /// mirroring the URL chip's Copy Link).
    @State private var copied = false

    var body: some View {
        HStack(spacing: 12) {
            HostFavicon(host: login.host)
            VStack(alignment: .leading, spacing: 2) {
                Text(login.displayName)
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(1)
                HStack(spacing: 6) {
                    Text(login.username ?? "-")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    if let revealed {
                        Text("·").foregroundStyle(.tertiary)
                        Text(revealed)
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                            .lineLimit(1)
                    }
                }
            }
            Spacer(minLength: 12)
            if hovering || revealed != nil {
                Button {
                    toggleReveal()
                } label: {
                    Image(systemName: revealed == nil ? "eye" : "eye.slash")
                        .font(.system(size: 12))
                        .contentTransition(.symbolEffect(.replace))
                        .frame(width: 24, height: 24)
                }
                .buttonStyle(IconButtonStyle(cornerRadius: 6))
                .help(revealed == nil ? "Reveal password" : "Hide password")

                Button {
                    copyPassword()
                } label: {
                    Image(systemName: copied ? "checkmark" : "doc.on.doc")
                        .font(.system(size: 12))
                        .contentTransition(.symbolEffect(.replace))
                        .frame(width: 24, height: 24)
                }
                .buttonStyle(IconButtonStyle(cornerRadius: 6, emphasizesLabel: false))
                .foregroundStyle(copied ? AnyShapeStyle(.green) : AnyShapeStyle(.secondary))
                .help("Copy password")
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 8)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(.quaternary)
                .opacity(hovering ? 0.6 : 0)
                .hoverFade(hovering)
        )
        .contentShape(.rect)
        .onHover { hovering = $0 }
        .contextMenu {
            Button("Edit…") {
                model.editing = EditorTarget(detail: model.loginDetail(id: login.id))
            }
            Divider()
            Button("Copy Username") { copy(login.username ?? "") }
            Button("Copy Password") { copyPassword() }
            Divider()
            Button("Delete", role: .destructive) { model.delete(id: login.id) }
        }
        .onTapGesture(count: 2) {
            model.editing = EditorTarget(detail: model.loginDetail(id: login.id))
        }
    }

    private func toggleReveal() {
        revealed = revealed == nil ? model.password(for: login.id) : nil
    }

    private func copyPassword() {
        guard let password = model.password(for: login.id) else { return }
        SecurePasteboard.copySecret(password)
        copied = true
        Task {
            try? await Task.sleep(for: .seconds(1))
            copied = false
        }
    }

    /// Non-secret copy (username). Passwords go through `SecurePasteboard`.
    private func copy(_ string: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(string, forType: .string)
    }
}

/// Site favicon (or a globe fallback) - shared by the vault list and the URL-bar
/// quick-access popover (`VaultQuickAccess`).
struct HostFavicon: View {
    let host: String
    @State private var image: NSImage?

    var body: some View {
        Group {
            if let image {
                Image(nsImage: image).resizable()
            } else {
                Image(systemName: "globe").foregroundStyle(.secondary)
            }
        }
        .frame(width: 18, height: 18)
        .task(id: host) {
            image = host.isEmpty ? nil : await FaviconLoader.shared.icon(for: host)
        }
    }
}

private struct EmptyState: View {
    let query: String
    var model: VaultModel?

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: query.isEmpty ? "key" : "magnifyingglass")
                .font(.system(size: 32, weight: .light))
                .foregroundStyle(.tertiary)
            Text(query.isEmpty ? "No saved passwords" : "No results for “\(query)”")
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(.secondary)
            if query.isEmpty {
                Text("Import your existing passwords to get started.")
                    .font(.system(size: 12))
                    .foregroundStyle(.tertiary)
                if let model {
                    Button {
                        importPasswords(model)
                    } label: {
                        Label("Import Passwords…", systemImage: "square.and.arrow.down")
                            .frame(maxWidth: 200)
                    }
                    .buttonStyle(.glass)
                    .disabled(model.busy)
                    .padding(.top, 4)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// ─── Add / edit sheet ─────────────────────────────────────────────────────────

/// Create or edit a login. Prefilled from `target.detail` when editing (a full
/// round-trip - a PUT replaces the cipher, so every field must be present).
private struct LoginEditor: View {
    @ObservedObject var model: VaultModel
    let target: EditorTarget
    @State private var draft = LoginDraft()

    private var isEditing: Bool { target.detail != nil }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(isEditing ? "Edit Login" : "Add Login")
                .font(.system(size: 16, weight: .semibold))

            VStack(spacing: 8) {
                field("Name", text: $draft.name)
                field("Username", text: $draft.username)
                secureField("Password", text: $draft.password)
                field("Website (https://…)", text: $draft.uri)
                field("TOTP secret (optional)", text: $draft.totp)
                field("Notes (optional)", text: $draft.notes)
            }

            HStack {
                if isEditing {
                    Button("Delete", role: .destructive) {
                        model.delete(id: target.detail!.id)
                        model.editing = nil
                    }
                    .disabled(model.busy)
                }
                Spacer()
                Button("Cancel") { model.editing = nil }
                    .keyboardShortcut(.cancelAction)
                Button(isEditing ? "Save" : "Add") {
                    model.save(id: target.detail?.id, draft: draft)
                    model.editing = nil
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(draft.name.trimmingCharacters(in: .whitespaces).isEmpty || model.busy)
            }
        }
        .padding(24)
        .frame(width: 380)
        .onAppear {
            if let d = target.detail {
                draft = LoginDraft(
                    name: d.name, username: d.username ?? "", password: d.password ?? "",
                    uri: d.uri ?? "", totp: d.totp ?? "", notes: d.notes ?? ""
                )
            }
        }
    }

    private func field(_ prompt: String, text: Binding<String>) -> some View {
        TextField(prompt, text: text)
            .textFieldStyle(.roundedBorder)
            .font(.system(size: 13))
    }

    private func secureField(_ prompt: String, text: Binding<String>) -> some View {
        SecureField(prompt, text: text)
            .textFieldStyle(.roundedBorder)
            .font(.system(size: 13))
    }
}

// ─── Model ──────────────────────────────────────────────────────────────────

/// One login as listed by `browser_vault_list` - metadata only, no password.
struct VaultLogin: Decodable, Identifiable {
    let id: String
    let name: String
    let username: String?
    let uris: [String]

    var displayName: String {
        if !name.isEmpty { return name }
        if let host = host.nonEmpty { return host }
        return username ?? "Login"
    }

    /// The host of the first URI, for favicon + display.
    var host: String {
        guard let uri = uris.first else { return "" }
        var s = uri
        if let range = s.range(of: "://") { s = String(s[range.upperBound...]) }
        if let slash = s.firstIndex(where: { $0 == "/" || $0 == "?" || $0 == "#" }) {
            s = String(s[..<slash])
        }
        if let at = s.lastIndex(of: "@") { s = String(s[s.index(after: at)...]) }
        if let colon = s.lastIndex(of: ":") { s = String(s[..<colon]) }
        return s.lowercased()
    }
}

extension String {
    fileprivate var nonEmpty: String? { isEmpty ? nil : self }
}

/// A login's full detail (incl. password + notes) for the editor, from
/// `browser_vault_login`.
struct LoginDetail: Decodable {
    let id: String
    let name: String
    let username: String?
    let password: String?
    let uri: String?
    let totp: String?
    let notes: String?
}

/// The add/edit sheet's target - a detail to edit, or `nil` fields for a new login.
struct EditorTarget: Identifiable {
    let detail: LoginDetail?
    var id: String { detail?.id ?? "__new__" }
}

/// Editable login fields.
struct LoginDraft {
    var name = ""
    var username = ""
    var password = ""
    var uri = ""
    var totp = ""
    var notes = ""
}

/// Run `body` with C-string pointers for each `strings` element (nested
/// `withCString`, so the pointers are valid for the call's duration).
func withCStrings<R>(_ strings: [String], _ body: ([UnsafePointer<CChar>]) -> R) -> R {
    func recurse(_ index: Int, _ acc: [UnsafePointer<CChar>]) -> R {
        if index == strings.count { return body(acc) }
        return strings[index].withCString { recurse(index + 1, acc + [$0]) }
    }
    return recurse(0, [])
}

/// Observable projection of the core vault. Reloaded when the surface opens.
@MainActor
final class VaultModel: ObservableObject {
    static let shared = VaultModel()

    /// The list filter. Ranking lives in the core (`browser_vault_search`), so a
    /// change re-queries rather than filtering `logins` here - the semantics stay
    /// identical to the URL-bar popover's search, and a second frontend inherits
    /// them instead of reinventing them (ADR-0049).
    @Published var query = "" {
        didSet {
            guard query != oldValue else { return }
            refreshFiltered()
        }
    }
    @Published private(set) var isConfigured = false
    @Published private(set) var isLocked = true
    @Published private(set) var logins: [VaultLogin] = []
    /// `logins` ranked against `query` by the core - what the list renders.
    @Published private(set) var filtered: [VaultLogin] = []
    /// A blocking op (setup / unlock / import) is in flight.
    @Published private(set) var busy = false
    /// The last setup/unlock error, shown inline.
    @Published var actionError: String?
    /// The pending two-factor challenge kind ("email"/"authenticator"/…), or nil.
    /// When set, the setup form switches to code entry and re-submits under it.
    @Published var twoFactor: String?
    /// Transient result of the last import (e.g. "Imported 12 logins").
    @Published var importStatus: String?
    /// The add/edit sheet target (nil = closed).
    @Published var editing: EditorTarget?

    /// Re-rank against the current query. An empty query lists the whole vault,
    /// so this one call covers the filtered and unfiltered states alike.
    private func refreshFiltered() {
        filtered = isLocked ? [] : Self.fetchSearch(query: query)
    }

    /// Refresh configured / lock state + list from the core.
    func reload() {
        isConfigured = browser_vault_is_configured()
        isLocked = browser_vault_is_locked()
        logins = isLocked ? [] : Self.fetchList()
        refreshFiltered()
        // Keep the URL-bar quick-access button in sync with lock-state changes
        // that happen here (unlock / lock / setup), not just on navigation.
        BrowserState.shared.refreshVaultMatches()
    }

    /// First-run setup / online login. Blocks (network + KDF), so the FFI call
    /// runs off the main actor. Three outcomes: unlocked (reload), a two-factor
    /// challenge (keep the form, switch to code entry - for email the code is
    /// already sent), or an error.
    func setup(server: String, email: String, password: String, code: String, provider: String) {
        guard !busy else { return }
        busy = true
        actionError = nil
        Task {
            let result = await Self.envelope5 { browser_vault_setup($0, $1, $2, $3, $4) }(
                server, email, password, code, provider)
            busy = false
            if result.ok {
                twoFactor = nil
                reload()
            } else if let kind = result.twoFactor {
                // A fresh challenge - prompt for the code (no error to show yet).
                twoFactor = kind
            } else {
                // Keep any challenge mode so a rejected code stays on code entry.
                actionError = result.error ?? "Something went wrong"
            }
        }
    }

    /// Re-dispatch the login two-factor email (the "Resend code" affordance).
    func resendEmailCode(email: String, password: String) {
        guard !busy else { return }
        busy = true
        actionError = nil
        Task {
            let result = await Self.envelope2(browser_vault_send_email_code)(email, password)
            busy = false
            if !result.ok { actionError = result.error ?? "Could not resend the code" }
        }
    }

    /// Offline unlock from cache.
    func unlock(password: String) {
        run { await Self.envelope1(browser_vault_unlock_offline)(password) }
    }

    /// Import a CSV or Bitwarden-JSON export file - **merges** into the vault
    /// (the core dedupes). Picks the FFI by extension.
    func importFile(_ url: URL) {
        guard let data = try? String(contentsOf: url, encoding: .utf8) else {
            importStatus = "Could not read \(url.lastPathComponent)"
            return
        }
        let isJSON = url.pathExtension.lowercased() == "json"
        busy = true
        actionError = nil
        importStatus = nil
        Task {
            let raw = await Task.detached {
                data.withCString { d -> String in
                    let ptr =
                        isJSON
                        ? browser_vault_import_bitwarden_json(d) : browser_vault_import_csv(d)
                    guard let ptr else { return "" }
                    defer { browser_string_free(ptr) }
                    return String(cString: ptr)
                }
            }.value
            busy = false
            let result = VaultEnvelope.parse(raw)
            if result.ok {
                let n = result.imported ?? 0
                importStatus =
                    n == 0 ? "Nothing new to import" : "Imported \(n) login\(n == 1 ? "" : "s")"
                reload()
            } else {
                importStatus = result.error ?? "Import failed"
            }
        }
    }

    /// Run a blocking, envelope-returning op off the main actor; on success reload.
    private func run(_ op: @escaping () async -> VaultEnvelope) {
        guard !busy else { return }
        busy = true
        actionError = nil
        Task {
            let result = await op()
            busy = false
            if result.ok {
                reload()
            } else {
                actionError = result.error ?? "Something went wrong"
            }
        }
    }

    /// Wrap a 5-arg C-string FFI call in a detached task returning its envelope.
    nonisolated private static func envelope5(
        _ call:
            @escaping (
                UnsafePointer<CChar>?, UnsafePointer<CChar>?, UnsafePointer<CChar>?,
                UnsafePointer<CChar>?, UnsafePointer<CChar>?
            ) -> UnsafeMutablePointer<CChar>?
    ) -> (String, String, String, String, String) async -> VaultEnvelope {
        { a, b, c, d, e in
            await Task.detached {
                withCStrings([a, b, c, d, e]) { p in
                    guard let raw = call(p[0], p[1], p[2], p[3], p[4]) else {
                        return VaultEnvelope(ok: false, error: "no response")
                    }
                    defer { browser_string_free(raw) }
                    return VaultEnvelope.parse(String(cString: raw))
                }
            }.value
        }
    }

    /// Wrap a 2-arg C-string FFI call in a detached task returning its envelope.
    nonisolated private static func envelope2(
        _ call:
            @escaping (UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
    ) -> (String, String) async -> VaultEnvelope {
        { a, b in
            await Task.detached {
                a.withCString { pa in
                    b.withCString { pb in
                        guard let raw = call(pa, pb) else {
                            return VaultEnvelope(ok: false, error: "no response")
                        }
                        defer { browser_string_free(raw) }
                        return VaultEnvelope.parse(String(cString: raw))
                    }
                }
            }.value
        }
    }

    /// Wrap a 1-arg C-string FFI call in a detached task returning its envelope.
    nonisolated private static func envelope1(
        _ call: @escaping (UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
    ) -> (String) async -> VaultEnvelope {
        { a in
            await Task.detached {
                a.withCString { pa in
                    guard let raw = call(pa) else {
                        return VaultEnvelope(ok: false, error: "no response")
                    }
                    defer { browser_string_free(raw) }
                    return VaultEnvelope.parse(String(cString: raw))
                }
            }.value
        }
    }

    func lock() {
        browser_vault_lock()
        reload()
    }

    /// Full detail for login `id` (fast, in-memory) - for prefilling the editor.
    func loginDetail(id: String) -> LoginDetail? {
        guard let raw = id.withCString({ browser_vault_login($0) }) else { return nil }
        defer { browser_string_free(raw) }
        return try? JSONDecoder().decode(LoginDetail.self, from: Data(String(cString: raw).utf8))
    }

    /// Create (`id == nil`) or update a login. Blocks (network) - runs off-main.
    func save(id: String?, draft: LoginDraft) {
        run {
            await Task.detached {
                withCStrings([
                    id ?? "", draft.name, draft.username, draft.password, draft.uri, draft.totp,
                    draft.notes,
                ]) { p in
                    let raw: UnsafeMutablePointer<CChar>? =
                        (id?.isEmpty == false)
                        ? browser_vault_update_login(p[0], p[1], p[2], p[3], p[4], p[5], p[6])
                        : browser_vault_add_login(p[1], p[2], p[3], p[4], p[5], p[6])
                    guard let raw else { return VaultEnvelope(ok: false, error: "no response") }
                    defer { browser_string_free(raw) }
                    return VaultEnvelope.parse(String(cString: raw))
                }
            }.value
        }
    }

    /// Delete login `id`. Blocks (network) - runs off-main.
    func delete(id: String) {
        run { await Self.envelope1(browser_vault_delete_login)(id) }
    }

    /// Fetch a login's password (fast, in-memory) - the one secret path.
    func password(for id: String) -> String? {
        guard let raw = id.withCString({ browser_vault_password($0) }) else { return nil }
        defer { browser_string_free(raw) }
        return String(cString: raw)
    }

    private static func fetchList() -> [VaultLogin] {
        guard let raw = browser_vault_list() else { return [] }
        defer { browser_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        return (try? JSONDecoder().decode([VaultLogin].self, from: data)) ?? []
    }

    /// Core-ranked search over the whole vault (empty query ⇒ everything).
    static func fetchSearch(query: String) -> [VaultLogin] {
        guard let raw = query.withCString({ browser_vault_search($0) }) else { return [] }
        defer { browser_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        return (try? JSONDecoder().decode([VaultLogin].self, from: data)) ?? []
    }
}

/// A `{"ok":…,"error":…,"imported":N,"two_factor":"…"}` FFI envelope.
private struct VaultEnvelope {
    let ok: Bool
    let error: String?
    let imported: Int?
    /// The two-factor challenge kind when the server asked for one (nil otherwise).
    let twoFactor: String?

    init(ok: Bool, error: String?, imported: Int? = nil, twoFactor: String? = nil) {
        self.ok = ok
        self.error = error
        self.imported = imported
        self.twoFactor = twoFactor
    }

    static func parse(_ string: String) -> VaultEnvelope {
        guard
            let data = string.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return VaultEnvelope(ok: false, error: "Could not reach the vault")
        }
        return VaultEnvelope(
            ok: object["ok"] as? Bool ?? false,
            error: object["error"] as? String,
            imported: object["imported"] as? Int,
            twoFactor: object["two_factor"] as? String
        )
    }
}
