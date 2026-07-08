import AppKit
import SwiftUI

/// The URL-bar vault quick-access button + current-site logins popover (P2-34,
/// ADR-0029) — the 1Password/Bitwarden toolbar idiom, adapted to sit beside the
/// address chip like the adblock shield (`URLBar`).
///
/// This is the **detection-independent** autofill path: it lists the logins the
/// core matched to the active tab's host (`browser_vault_lookup`, mirrored on
/// `BrowserState.vaultMatches`) and offers, per entry, **Fill** (core-initiated
/// injection — `browser_vault_fill_active_tab`), **Copy username**, and **Copy
/// password**. Copy works on *any* page regardless of form detection — the
/// reliable escape hatch when the auto-picker can't find the fields (e.g. a
/// two-step `google.com` sign-in). Passwords cross only on the explicit Fill/Copy
/// gesture; the list carries metadata only.

// ─── The button ────────────────────────────────────────────────────────────────

enum VaultKeyButtonPlacement {
    /// Nested inside the sidebar address chip (`IconButtonStyle`).
    case sidebar
    /// Flanking the in-page address pill as its own Liquid Glass control.
    case viewport
}

struct VaultKeyButton: View {
    let host: String
    @Binding var isPresented: Bool
    var placement: VaultKeyButtonPlacement = .sidebar
    @ObservedObject private var state = BrowserState.shared

    /// Filled + accent-tinted when this site has saved logins (the valuable
    /// case); a plain outline otherwise (locked, or no match — reachable on hover).
    private var hasMatches: Bool { !state.vaultLocked && !state.vaultMatches.isEmpty }

    @ViewBuilder var body: some View {
        switch placement {
        case .sidebar:
            sidebarBody
        case .viewport:
            viewportBody
        }
    }

    private var sidebarBody: some View {
        Button {
            isPresented.toggle()
        } label: {
            Image(systemName: hasMatches ? "key.horizontal.fill" : "key.horizontal")
                .font(.system(size: 12, weight: .semibold))
                .contentTransition(.symbolEffect(.replace))
                .foregroundStyle(
                    hasMatches ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(.secondary)
                )
                .frame(height: 18)
                .padding(.horizontal, 2)
        }
        .buttonStyle(IconButtonStyle(cornerRadius: 5, emphasizesLabel: false))
        .help(vaultHelp)
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            VaultQuickPopover(host: host, isPresented: $isPresented)
        }
    }

    /// `onTapGesture` + `contentShape` — same hit-test path as the in-page address
    /// pill. A `Button` does not register under `PassthroughHostingView`.
    private var viewportBody: some View {
        Image(systemName: hasMatches ? "key.horizontal.fill" : "key.horizontal")
            .font(.system(size: 13, weight: .semibold))
            .contentTransition(.symbolEffect(.replace))
            .foregroundStyle(
                hasMatches ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(.secondary)
            )
            .frame(
                width: ViewportURLBarMetrics.pillHeight, height: ViewportURLBarMetrics.pillHeight
            )
            .background { ViewportGlassCapsule() }
            .contentShape(Capsule())
            .onTapGesture { isPresented.toggle() }
            .help(vaultHelp)
            .popover(isPresented: $isPresented, arrowEdge: .bottom) {
                VaultQuickPopover(host: host, isPresented: $isPresented)
            }
    }

    private var vaultHelp: String {
        if state.vaultLocked { return "Vault locked — click to unlock" }
        switch state.vaultMatches.count {
        case 0: return "No saved logins for this site"
        case 1: return "1 saved login for this site"
        default: return "\(state.vaultMatches.count) saved logins for this site"
        }
    }
}

// ─── The popover ─────────────────────────────────────────────────────────────

private struct VaultQuickPopover: View {
    let host: String
    @Binding var isPresented: Bool
    @ObservedObject private var state = BrowserState.shared

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                HostFavicon(host: host)
                Text(host.isEmpty ? "This site" : host)
                    .font(.headline)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }

            if state.vaultLocked {
                locked
            } else if state.vaultMatches.isEmpty {
                empty
            } else {
                Divider()
                ScrollView {
                    VStack(spacing: 2) {
                        ForEach(state.vaultMatches) { login in
                            VaultQuickRow(login: login, isPresented: $isPresented)
                        }
                    }
                }
                .frame(maxHeight: 264)
            }
        }
        .padding(14)
        .frame(width: 300)
    }

    private var locked: some View {
        VaultQuickUnlock(isPresented: $isPresented)
    }

    private var empty: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("No saved logins for this site.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
            Button {
                BrowserState.shared.openVault()
                isPresented = false
            } label: {
                Label("Open Passwords", systemImage: "key.horizontal")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.bordered)
        }
    }
}

// ─── Inline unlock (vault locked) ───────────────────────────────────────────

/// Master-password unlock in the popover — same offline path as the Passwords
/// page, without leaving the tab.
private struct VaultQuickUnlock: View {
    @Binding var isPresented: Bool
    @ObservedObject private var model = VaultModel.shared
    @ObservedObject private var state = BrowserState.shared
    @State private var password = ""
    @FocusState private var passwordFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("Your vault is locked.", systemImage: "lock.fill")
                .font(.subheadline)
                .foregroundStyle(.secondary)

            SecureField("Master password", text: $password)
                .textFieldStyle(.plain)
                .font(.system(size: 13))
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .glassEffect(.regular, in: .capsule)
                .focused($passwordFocused)
                .onSubmit { unlock() }

            if let error = model.actionError {
                Text(error)
                    .font(.system(size: 11))
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Button(action: unlock) {
                if model.busy {
                    ProgressView().controlSize(.small).frame(maxWidth: .infinity)
                } else {
                    Text("Unlock").frame(maxWidth: .infinity)
                }
            }
            .buttonStyle(.borderedProminent)
            .disabled(password.isEmpty || model.busy)

            Button {
                BrowserState.shared.openVault()
                isPresented = false
            } label: {
                Text("Open Passwords")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.bordered)
        }
        .onAppear { focusPassword() }
        .onChange(of: isPresented) { _, open in
            if open { focusPassword() } else { password = "" }
        }
        .onChange(of: state.vaultLocked) { _, locked in
            if !locked { password = "" }
        }
    }

    private func unlock() {
        model.unlock(password: password)
    }

    private func focusPassword() {
        DispatchQueue.main.async { passwordFocused = true }
    }
}

// ─── One login row (Fill · Copy username · Copy password) ────────────────────

private struct VaultQuickRow: View {
    let login: VaultLogin
    @Binding var isPresented: Bool

    @State private var hovering = false
    @State private var copiedUser = false
    @State private var copiedPassword = false

    var body: some View {
        HStack(spacing: 10) {
            HostFavicon(host: login.host)
            VStack(alignment: .leading, spacing: 1) {
                Text(login.displayName)
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(1)
                Text(login.username ?? "—")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            if login.username?.isEmpty == false {
                iconButton(
                    copiedUser ? "checkmark" : "person", tint: copiedUser ? .green : .secondary,
                    help: "Copy username"
                ) {
                    copy(login.username ?? "")
                    copiedUser = true
                    resetFlags()
                }
            }
            iconButton(
                copiedPassword ? "checkmark" : "key", tint: copiedPassword ? .green : .secondary,
                help: "Copy password"
            ) {
                copyPassword()
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 7)
        .background(
            RoundedRectangle(cornerRadius: 7)
                .fill(.quaternary)
                .opacity(hovering ? 0.6 : 0)
        )
        .contentShape(.rect(cornerRadius: 7))
        // The row itself fills — the primary action, like picking an entry in
        // Bitwarden. The copy buttons are nested actions, so (as with TabRow /
        // the URL chip) the row uses onTapGesture to keep their taps unambiguous.
        .onTapGesture { fill() }
        .onHover { hovering = $0 }
        .help("Fill this login")
    }

    private func iconButton(
        _ symbol: String, tint: Color, help: String, _ action: @escaping () -> Void
    )
        -> some View
    {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 12))
                .contentTransition(.symbolEffect(.replace))
                .frame(width: 22, height: 22)
        }
        .buttonStyle(IconButtonStyle(cornerRadius: 6, emphasizesLabel: false))
        .foregroundStyle(tint)
        .help(help)
    }

    private func fill() {
        BrowserState.shared.fillActiveTab(id: login.id)
        isPresented = false
    }

    private func copyPassword() {
        guard let password = VaultModel.shared.password(for: login.id) else { return }
        // Auto-clear after 30s (Bitwarden-style), but only if the user hasn't
        // copied something else since — never clobber a later clipboard.
        copy(password, clearAfter: 30)
        copiedPassword = true
        resetFlags()
    }

    private func copy(_ string: String, clearAfter seconds: Double? = nil) {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(string, forType: .string)
        guard let seconds else { return }
        let stamp = pasteboard.changeCount
        Task {
            try? await Task.sleep(for: .seconds(seconds))
            if NSPasteboard.general.changeCount == stamp {
                NSPasteboard.general.clearContents()
            }
        }
    }

    /// Clear the copy-confirmation checkmarks after a beat (matches VaultRow /
    /// the URL chip's Copy-Link feedback).
    private func resetFlags() {
        Task {
            try? await Task.sleep(for: .seconds(1))
            copiedUser = false
            copiedPassword = false
        }
    }
}
