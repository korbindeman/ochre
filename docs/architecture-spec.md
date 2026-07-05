# Native-Chrome Browser — Architecture Spec

A minimal, fast, cross-platform browser: **Arc's interaction model, native OS chrome.** Chromium engine via CEF, a shared Rust core, and per-platform native UI (SwiftUI + Liquid Glass on macOS, WinUI 3 + Fluent on Windows).

---

## 1. Decision & rationale

**Architecture A: shared Rust core + native UI per platform.**

The requirement that forces this: native Liquid Glass **controls and animations** (the Spotlight-style Cmd+T morph, glass buttons/inputs/sliders), native icons, native feel. Those materials and their morph/expand animations are system-provided and only reachable through the native UI frameworks (`glassEffect`, `GlassEffectContainer`, `glassEffectID` on macOS; Mica/Acrylic + Composition animations on Windows). A custom GPU renderer (Vello) can only approximate them — the opposite of the goal. Native controls also bring platform behavior and accessibility for free.

**Vello is out for the chrome.** With web content living in a native CEF view and the chrome being native glass, there is no surface left for a custom renderer.

**What is NOT lost:** Arc's interaction model is entirely view logic and is fully reproducible natively (see §6). "Snappy" comes from Core Animation / SwiftUI springs and WinUI Composition animations, which are native and excellent.

**Accepted cost:** two thin native UI layers (Swift, C#/XAML) atop one shared Rust core. Larger surface than a single codebase, but the only route to real native glass — and the most agent-friendly targets available.

---

## 2. Non-negotiables

- Arc UX preserved exactly: vertical tabs, Arc-like spaces, sidebar, Cmd+T center command bar.
- Native chrome: real Liquid Glass on macOS, real Mica/Acrylic + Fluent on Windows.
- Native controls (buttons, inputs, sliders) and native icons (SF Symbols on macOS, Segoe Fluent Icons on Windows).
- Native glass animations, especially the Cmd+T command bar expanding Spotlight-style.
- Snappy sidebar/tab animations (native springs).
- Chromium engine (passkeys, DevTools, DRM), extensions **not** required — adblock and vault are native (§9, §10).

---

## 3. High-level architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Native UI (per platform, thin)                              │
│   macOS:  SwiftUI + Liquid Glass + SF Symbols                │
│   Windows: WinUI 3 + Mica/Acrylic + Segoe Fluent Icons       │
│   - sidebar / vertical tabs / spaces                         │
│   - toolbar, native controls                                 │
│   - Cmd+T glass command bar (native morph)                   │
│   - hosts the web view as a native subview                   │
└───────────────▲──────────────────────────────┬──────────────┘
                │  FFI (UniFFI / swift-bridge;   │  native view host
                │  windows-rs / C# bindings)     │  (NSViewRepresentable /
                │                                │   HWND / SwapChainPanel)
┌───────────────┴────────────────┐   ┌──────────▼──────────────┐
│  Shared Rust core (the brain)  │   │  CEF (Chromium)         │
│   - CEF wrapper / lifecycle    │──▶│   - one native view per │
│   - tab / space / session model│   │     tab (windowed)      │
│   - adblock engine on requests │   │   - DevTools + CDP      │
│   - password vault             │   │   - passkeys / WebAuthn │
│   - sync client                │   └─────────────────────────┘
│   - CDP automation surface     │
└────────────────────────────────┘
```

**Split of responsibility:** the Rust core owns all state and logic; the native layers are view + binding only. This keeps ~80% of the code in one Rust codebase and duplicates only the thin presentation layer.

---

## 4. Shared Rust core

Single Cargo workspace. Modules:

| Module | Responsibility | Key crates |
|---|---|---|
| `engine` | CEF init, browser/host lifecycle, per-tab native view handles, resource-request interception | `cef` (tauri-apps/cef-rs) **or** cef-ui (hytopiagg) |
| `tabs` | Tab + space + session model, ordering, persistence | — |
| `adblock` | Filter-list load/update, network + cosmetic filtering on the CEF request path | `adblock` (brave/adblock-rust) |
| `vault` | Bitwarden-compatible client + crypto, autofill lookup | rbw-derived logic / `rwarden` (see §10) |
| `sync` | Tab/space/settings sync client (separate service, §12) | `reqwest`, crypto crates |
| `automation` | Thin wrapper over CEF's CDP endpoint for user-facing automations | CDP over remote-debugging port |
| `ffi` | The exported API surface for the native UIs | `uniffi` / `swift-bridge` |

**Engine notes**
- CEF uses **prebuilt binary distributions** (Spotify cefbuilds) — no Chromium compilation. The `-sys` crate builds only `libcef_dll_wrapper` via cmake.
- **Windowed rendering** (not OSR): each tab is a native `NSView` / child `HWND` that the native UI embeds. Simpler and faster than compositing OSR buffers; the chrome and web content stay in separate rectangles, which also sidesteps most view-layering pain.
- Passkeys / WebAuthn / FedCM work (core Chromium). **Widevine/DRM** requires a CEF build that includes it — decide early if Netflix/Spotify-web matter.

---

## 5. FFI boundary

Expose a stable, coarse-grained API from the Rust core (commands + an event/observation stream). Keep it chatty-averse: the native UI observes core state and issues intents.

- **macOS:** `UniFFI` (Rust→Swift) for the data/command surface, or `swift-bridge` where the boundary needs richer types / bidirectional calls. The web view is embedded by handing the CEF-created `NSView` into SwiftUI via `NSViewRepresentable`.
- **Windows:** if the UI is `windows-reactor` (Rust, recommended — see §8), there is **no FFI boundary** — the UI is another Rust crate calling the core in-process. If you fall back to **C#/WinUI 3**, bridge to the core via a C ABI (UniFFI's community C# bindings or `csbindgen`). Either way the web view is a child `HWND` (or `SwapChainPanel`/`HwndHost`-style hosting).

Exposed surface (illustrative): `open_tab`, `close_tab`, `reorder_tab`, `switch_space`, `navigate`, `command_bar_query`, `vault_lookup(host)`, `toggle_devtools`, plus an event stream: `TabTitleChanged`, `TabFaviconChanged`, `LoadStateChanged`, `NavigationCommitted`.

---

## 6. Arc UX → native mapping (proof nothing is lost)

| Arc behavior | macOS (SwiftUI) | Windows (WinUI 3) |
|---|---|---|
| Vertical tab list | `LazyVStack`/`List` rows, drag-to-reorder, `.animation(.spring)` | `ItemsRepeater`/`ListView`, drag-reorder, implicit/Composition animations |
| Spaces (switchable tab sets) | Segmented/space switcher bound to core `space` state | `SelectorBar`/pivot bound to core state |
| Sidebar collapse/expand | Spring width animation + `matchedGeometryEffect` | Composition size animation |
| Cmd+T command bar (center, Spotlight-like) | Centered glass overlay, `GlassEffectContainer` + `glassEffectID` morph, `.glassEffect(.regular.interactive())` | Centered Acrylic surface, `ConnectedAnimation` / Composition expand |
| Hover previews / tab affordances | Native hover + spring transitions | Pointer-over states + Composition |
| Command palette actions | SwiftUI list over the glass surface | WinUI list over the Acrylic surface |

The interaction model is view logic bound to the shared core; the "feel" is native animation. No custom rendering required anywhere.

---

## 7. macOS UI (SwiftUI + Liquid Glass)

- **Materials:** `NSGlassEffectView` / SwiftUI `.glassEffect()`. Per Apple guidance, glass belongs on the **top functional layer** (toolbar, sidebar, floating controls), never on content rows. Put content *inside* the glass (`contentView` / `.glassEffect` on the control), don't stack glass as a sibling backdrop.
- **Controls:** native buttons/inputs/sliders — they inherit Liquid Glass automatically when built with the current SDK.
- **Icons:** SF Symbols via `Image(systemName:)`.
- **Window:** transparent title bar / full-size content; sidebar + toolbar as glass; web view (CEF `NSView`) in the content region via `NSViewRepresentable`.
- **Animations:** SwiftUI springs, `matchedGeometryEffect`, and for glass morphing `glassEffectID` + a `@Namespace` inside a `GlassEffectContainer`.
- **Visual style — slim, content-first (Safari-like):** Arc's *interaction* model, not Arc's chrome. Web content takes maximum real estate; **no Arc-style thick inset frame**. Chrome is a thin glass sidebar + minimal top controls; no persistent full-width address bar (URL via Cmd+T). Target the native macOS 26 Safari look — content full-bleed with slim glass floating over it (glass lensing live content) — with the fallback of content flush beside a slim glass sidebar if the over-content compositing seam is troublesome. Windows mirrors this philosophy with Mica/Fluent.

## 8. Windows UI (WinUI 3 + Fluent)

**Layer split — decide the control layer, not the whole thing:**

- **Window + Mica/Acrylic material + DWM + CEF `HWND` hosting → pure `windows-rs`, always.** None of this needs C#. `Window.SystemBackdrop` = `MicaBackdrop` (main window; `Mica Alt` for a tabbed title bar); `DesktopAcrylicBackdrop`/`AcrylicBrush` **sparingly** for transient surfaces (command bar, menus) — Mica is nearly free (samples wallpaper once), Acrylic is heavier (continuous sampling). Backdrop shows only where layers above are transparent. CEF web view = child `HWND` hosted in the tree.
- **Control / declarative-UI layer → pick one:**

| Path | Controls | Language | Notes |
|---|---|---|---|
| **`windows-reactor`** (recommended to try) | native WinUI 3 Fluent | **Rust** | React-style declarative (hooks, components, 55+ widgets, Composition + Animation Manager, theming, a11y), first-party (windows-rs team), ~3 MB binaries. **New (May 2026)** → coverage gaps possible, thin agent training data (feed agents its gallery/docs). |
| C# + WinUI 3 + XAML (fallback) | native WinUI 3 Fluent | C# | Mature, huge agent corpus, full tooling (XAML compiler, hot reload, Community Toolkit). Reintroduces a Rust↔C# FFI boundary. |
| raw `windows-rs` imperative WinUI | native but brutal | Rust | MS doesn't recommend WinUI-without-XAML for full apps. Avoid. |

- **Icons:** Segoe Fluent Icons via `FontIcon`/`SymbolIcon`.
- **Animations:** implicit animations, `ConnectedAnimation`, Composition API for the command-bar expand (reachable from both `windows-reactor` and C#).
- **Reference:** Edge's "Project Mai" is essentially this target (Mica, floating panels, rounded corners, vertical tabs, opaque address bar/active tab) — a good north star.

**Decision:** try `windows-reactor` first (keeps the whole app in Rust; see §18 monorepo payoff). De-risk it in the step-0 spike — a Fluent panel + the Cmd+T expand + CEF `HWND` embed. Fall back to C#/WinUI **only for the control layer** if it can't cover them; window/material/CEF stay `windows-rs` regardless.

---

## 9. The Cmd+T command bar

Cross-platform behavior: global `Cmd/Ctrl+T` → a centered overlay (Spotlight position), text field focused, results list (open tabs, history, URL/search). Escape/click-away dismisses.

- **macOS:** centered `GlassEffectContainer` holding a glass search field + results, animated in with the interactive glass morph (`glassEffectID` transition) so it expands like Spotlight.
- **Windows:** centered Acrylic surface, expanded via `ConnectedAnimation`/Composition.

The bar queries the Rust core (`command_bar_query`) which unifies tabs/history/vault suggestions.

---

## 10. Password vault

**Do not build on Bitwarden's `sdk-internal`** — its license forbids using it to build non-Bitwarden apps. Two license-clean routes:

- **rbw** (doy) — mature, feature-complete unofficial Bitwarden client in Rust (auth, vault crypto, sync, agent). CLI+agent-shaped; vendor/adapt its client + crypto logic. MIT.
- **rwarden + rwarden_crypto** (nn1ks) — more library-shaped API client with crypto split out; dual MIT/Apache; less actively maintained.

Talk to bitwarden.com or a self-hosted **Vaultwarden**. Crypto per Bitwarden's security whitepaper (KDF PBKDF2/Argon2id → master key → stretched key → protected symmetric key; AES-256-CBC + HMAC-SHA256) using standard crates (`argon2`, `aes`, `hmac`).

Caveats: rbw doesn't support passkey/Duo as a **login** 2FA (TOTP/Yubico OTP fine); bitwarden.com flags CLI traffic as bot traffic, so device registration via personal API key is needed. Autofill surfaces through the core (`vault_lookup(host)`), consumed by the native UI.

---

## 11. Ad blocking

Native, no extension needed. Use the **`adblock`** crate (brave/adblock-rust — powers Brave, now also in Firefox).

- Hook it into the CEF **resource-request handler** for network blocking; inject cosmetic-filter CSS/scriptlets per page.
- Consumes uBlock Origin / EasyList / EasyPrivacy syntax directly (uBO-compatible resources).
- **Better than uBO Lite:** not bound by MV3 declarativeNetRequest caps — full cosmetic filtering + scriptlet injection, no rule-count limits.
- Disable `embedded-domain-resolver` when embedding to avoid binary bloat; enable single-thread perf features.

---

## 12. Sync (separate service)

Not a browser feature — its own project. Auth, a sync server, conflict resolution, ideally E2E encryption. Sync tabs/spaces/settings. Treat as the highest-risk external dependency and build after the browser stands alone.

---

## 13. DevTools & automation

Both come from CEF (Chromium), independent of the UI.

- **DevTools:** full, real Chrome DevTools (all panels). `CefBrowserHost::ShowDevTools()` — popup or docked into a hosted view. If the Rust binding hasn't surfaced `show_dev_tools`, it's a one-line FFI call over `-sys`.
- **Automation:** enable `--remote-debugging-port` → full CDP. Same protocol as DevTools, so DevTools and web-automation are one surface.

---

## 14. Accessibility

A bonus of going native: the chrome gets platform accessibility **for free** — VoiceOver / NSAccessibility on macOS, Narrator / UI Automation on Windows — because you're using native controls. No AccessKit or custom a11y tree needed (which a Vello chrome would have required).

---

## 15. Risks — prototype these first

1. **CEF ↔ native embedding**: keyboard/focus routing between chrome and the embedded web view; window activation; per-tab view lifecycle.
2. **Cmd+T glass morph feel**: get the Spotlight-like expand right early — it's the signature interaction.
3. **Passkeys**: binding WebAuthn to the platform authenticator (Windows Hello / Touch ID) needs correct entitlements/config — validate in week one.
4. **Widevine/DRM**: only if you need protected video; requires a CEF build that includes it.
5. **Chromium update treadmill**: CEF tracks Chromium branches; budget recurring effort to bump CEF for security patches. This is the ongoing maintenance tax — plan for it.

---

## 16. Suggested build order

0. **Spike (de-risk the foundation).** cef-rs/cef-ui "one tab in a native window." Confirm: DevTools opens, a passkey login works, a DRM video plays (if needed), `adblock` blocks on the request path. Everything downstream depends on this passing.
1. **Rust core.** `engine` + `tabs`/session model + `adblock` + CDP surface + `ffi`.
2. **macOS shell.** SwiftUI window, sidebar + vertical tabs + spaces, CEF via `NSViewRepresentable`, Cmd+T glass command bar.
3. **Vault.** rbw-derived client wired into the command bar + autofill.
4. **Windows shell.** WinUI 3 mirror of the macOS shell (Mica/Acrylic, Fluent, Segoe icons, Composition animations).
5. **Sync service.** Separate; last.

---

## 17. Component summary

| Concern | Choice |
|---|---|
| Engine | CEF via `cef` (tauri-apps/cef-rs) or cef-ui (hytopiagg), prebuilt binaries, windowed rendering |
| Core language | Rust (single workspace) |
| macOS UI | SwiftUI + Liquid Glass + SF Symbols |
| Windows UI | `windows-reactor` (native WinUI 3 in Rust, recommended) or C#/WinUI 3 fallback; + Mica/Acrylic + Segoe Fluent Icons; window/material/CEF always `windows-rs` |
| Rust↔Swift | UniFFI / swift-bridge |
| Rust↔C# | None if `windows-reactor`; else UniFFI (C#) / csbindgen |
| Ad blocking | `adblock` (brave/adblock-rust) on the CEF request path |
| Vault | rbw-derived / `rwarden`; Bitwarden or Vaultwarden backend |
| DevTools / automation | CEF `ShowDevTools` + CDP (`--remote-debugging-port`) |
| Accessibility | Native (free via native controls) |
| Sync | Separate service |

---

## 18. Monorepo & platform projects

All three targets live in one git repo. Rust workspace at the root; native app projects as subdirectories.

```
browser/
├─ Cargo.toml                # Rust workspace
├─ crates/
│  ├─ core/                  # engine, tabs, adblock, vault, sync
│  ├─ ffi/                   # UniFFI + C ABI exports
│  └─ agent/                 # ACP host, tool exposure, virtual FS, permissions
├─ apps/
│  ├─ macos/
│  │  ├─ project.yml         # XcodeGen spec — SOURCE OF TRUTH (text, agent-edited)
│  │  ├─ Browser.xcodeproj   # GENERATED, disposable, gitignored
│  │  └─ Sources/            # SwiftUI
│  └─ windows/               # cargo crate (windows-reactor, native WinUI 3 in Rust)
│     └─ src/                # — no .sln/.csproj, no dotnet toolchain
│                            #   (fallback: C# .sln/.csproj + Sources/ if reactor falls short)
├─ bindings/                 # generated Swift (+ C# only if the C# fallback is used)
└─ scripts/                  # Makefile / justfile build orchestration
```

**Two languages, not three (if `windows-reactor` holds up):** the Windows app becomes an ordinary workspace crate built with `cargo build` — no separate C# project or dotnet toolchain — so agents drive it with the same commands as the core. Only the macOS app stays non-Rust (Liquid Glass's morph APIs are SwiftUI-only). C#/WinUI returns only as the Windows control-layer fallback.

**Xcode project handling (critical for agents):** never let agents hand-edit `.pbxproj` (opaque, merge-hostile). Use **XcodeGen** (`project.yml`) or **Tuist** to generate `.xcodeproj` from a plaintext spec. Agents edit the spec; a script regenerates the project. You initialize once (or generate); the durable artifact is text. SwiftPM is an alternative but Xcode projects handle app targets/signing/entitlements more smoothly.

**Headless CLI build paths (no IDE required):**
- Rust: `cargo build` / `cargo test`
- FFI: `cargo run --bin uniffi-bindgen` → regen `bindings/`
- macOS: `xcodegen generate` → `xcodebuild -scheme Browser build` (CLT, no GUI)
- Windows: `cargo build -p windows-app` (windows-reactor) — or `dotnet build Browser.sln` if using the C# fallback

Wrap in `scripts/` (`just build-macos`, `just build-windows`) so agents get pass/fail without an IDE. Constraint: `xcodebuild` is macOS-only, WinUI is Windows-only — full builds need both OSes or per-OS CI runners; each platform's agent has a complete headless path.

---

## 19. Agent layer (future)

Lives in `crates/agent`, over the same core.

- **ACP (Agent Client Protocol):** host local agents behind a standardized client↔agent boundary; the SwiftUI/WinUI chat panel speaks ACP so you're not locked to one agent backend.
- **Browser-as-tool:** CEF + CDP (§13) already provides navigate / extract / download / screenshot — expose as an MCP/ACP tool. "Pull in sites via the real browser" is near-free.
- **Virtual folder:** a sandboxed working dir the agent reads/writes; downloads route here, artifacts for the user are produced here.
- **Permission model:** one consent surface, escalating scopes — virtual folder → named paths → shell exec → full system. The in-sandbox vs. real-computer boundary is a hard, auditable line; that boundary is the entire safety story for full-computer access.
