# Architecture

The structural spec: **Arc's interaction model, native OS chrome.** A minimal, fast, cross-platform browser - Chromium engine via CEF, a shared Rust core, and per-platform native UI (SwiftUI + Liquid Glass on macOS, WinUI 3 + Fluent on Windows). §-numbered; other docs cite these sections as `spec §N`.

---

## 1. Decision & rationale

**Architecture A: shared Rust core + native UI per platform.**

The requirement that forces this: native Liquid Glass **controls and animations** (the Spotlight-style Cmd+T morph, glass buttons/inputs/sliders), native icons, native feel. Those materials and their morph/expand animations are system-provided and only reachable through the native UI frameworks (`glassEffect`, `GlassEffectContainer`, `glassEffectID` on macOS; Mica/Acrylic + Composition animations on Windows). A custom GPU renderer (Vello) can only approximate them - the opposite of the goal. Native controls also bring platform behavior and accessibility for free.

**Vello is out for the chrome.** With web content living in a native CEF view and the chrome being native glass, there is no surface left for a custom renderer.

**What is NOT lost:** Arc's interaction model is entirely view logic and is fully reproducible natively (see §6). "Snappy" comes from Core Animation / SwiftUI springs and WinUI Composition animations, which are native and excellent.

**Accepted cost:** two thin native UI layers (Swift, C#/XAML) atop one shared Rust core. Larger surface than a single codebase, but the only route to real native glass - and the most agent-friendly targets available.

---

## 2. Non-negotiables

- Arc UX preserved exactly: vertical tabs, Arc-like spaces, sidebar, Cmd+T center command bar.
- Native chrome: real Liquid Glass on macOS, real Mica/Acrylic + Fluent on Windows.
- Native controls (buttons, inputs, sliders) and native icons (SF Symbols on macOS, Segoe Fluent Icons on Windows).
- Native glass animations, especially the Cmd+T command bar expanding Spotlight-style.
- Snappy sidebar/tab animations (native springs).
- Chromium engine (passkeys, DevTools, DRM), extensions **not** required - adblock and vault are native (§9, §10).

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
| `tabs` | Tab + space + session model, ordering, persistence | - |
| `adblock` | Filter-list load/update, network + cosmetic filtering on the CEF request path | `adblock` (brave/adblock-rust) |
| `tweaks` | Per-site page customization (Snip / Restyle / Script): rule store, URL scoping, import/export; injects via the engine's cosmetic path (§20) | - |
| `commands` | User-defined browser automation (Commands, ADR-0034): registry, TS runtime, `CommandContext` API, permissions; surfaces in ⌘T bar + ⌘K palette | embedded JS runtime (TBD) |
| `vault` | Bitwarden-compatible client + crypto, autofill lookup | rbw-derived logic / `rwarden` (see §10) |
| `sync` | Tab/space/settings sync client (separate service, §12) | `reqwest`, crypto crates |
| `cdp` | Loopback consent-gated CDP port for dev/agent automation (spec §13) | CEF `--remote-debugging-port` |
| `ffi` | The exported API surface for the native UIs | `uniffi` / `swift-bridge` |

**Engine notes**
- CEF uses **prebuilt binary distributions** (Spotify cefbuilds) - no Chromium compilation. The `-sys` crate builds only `libcef_dll_wrapper` via cmake.
- **Windowed rendering** (not OSR): each tab is a native `NSView` / child `HWND` that the native UI embeds. Simpler and faster than compositing OSR buffers; the chrome and web content stay in separate rectangles, which also sidesteps most view-layering pain.
- Passkeys / WebAuthn / FedCM work (core Chromium). **Widevine/DRM** requires a CEF build that includes it - decide early if Netflix/Spotify-web matter.

---

## 5. FFI boundary

Expose a stable, coarse-grained API from the Rust core (commands + an event/observation stream). Keep it chatty-averse: the native UI observes core state and issues intents.

- **macOS:** the web view is embedded by handing the CEF-created `NSView` into SwiftUI via `NSViewRepresentable`. The data/command surface is a **hand-rolled coarse C ABI** - `crates/ffi/src/lib.rs` (~40+ `#[no_mangle]` fns) plus a C function-pointer event callback into Swift. The `UniFFI`/`swift-bridge` in the plan is **aspirational, not wired** - the shipped surface is the C ABI (it works well enough that the bridge dep was never added; revisit only if the boundary needs richer bidirectional types).
- **Windows:** if the UI is `windows-reactor` (Rust, recommended - see §8), there is **no FFI boundary** - the UI is another Rust crate calling the core in-process (the whole surface below is direct Rust). If you fall back to **C#/WinUI 3**, bind the same C ABI (`csbindgen`, or UniFFI's community C# bindings). Either way the web view is a child `HWND` (or `SwapChainPanel`/`HwndHost`-style hosting).

**The observation model (ADR-0005) - bind this, not a fine-grained event per change.** Hot, high-frequency per-tab signals stay fine-grained events (`TabTitleChanged`, `TabFaviconChanged`, `LoadStateChanged`, `NavigationCommitted`, `TabThemeColorChanged`); **every structural change** (create/close/reorder/pin/folder/space CRUD/transform) emits **one coarse `StructureChanged`**, on which the frontend **re-pulls a JSON snapshot** (`browser_structure_json`) and rebinds its view model. So a Windows frontend implements two things: a handful of hot-event handlers, and one snapshot decode. The illustrative commands (`open_tab`, `close_tab`, `switch_space`, `navigate`, `command_bar_query`, `vault_lookup(host)`, `toggle_devtools`, …) are all in the C ABI; **`crates/ffi/src/lib.rs` is the authoritative manifest** - treat it as the build-spec for the binding layer (the parity ledger's core rows should ultimately anchor to it, `parity` · ADR-0007).

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
- **Controls:** native buttons/inputs/sliders - they inherit Liquid Glass automatically when built with the current SDK.
- **Icons:** SF Symbols via `Image(systemName:)`.
- **Window:** transparent title bar / full-size content; sidebar + toolbar as glass; web view (CEF `NSView`) in the content region via `NSViewRepresentable`.
- **Animations:** SwiftUI springs, `matchedGeometryEffect`, and for glass morphing `glassEffectID` + a `@Namespace` inside a `GlassEffectContainer`.
- **Visual style:** slim, content-first, Safari-like - full rules live in `style-guide §3` (source of truth). In short: content full-bleed, **no Arc-style thick inset frame**, chrome is a thin glass sidebar + minimal top controls, URL via Cmd+T. Windows mirrors this with Mica/Fluent.

## 8. Windows UI (WinUI 3 + Fluent)

**Layer split - decide the control layer, not the whole thing:**

- **Window + Mica/Acrylic material + DWM + CEF `HWND` hosting → pure `windows-rs`, always.** None of this needs C#. `Window.SystemBackdrop` = `MicaBackdrop` (main window; `Mica Alt` for a tabbed title bar); `DesktopAcrylicBackdrop`/`AcrylicBrush` **sparingly** for transient surfaces (command bar, menus) - Mica is nearly free (samples wallpaper once), Acrylic is heavier (continuous sampling). Backdrop shows only where layers above are transparent. CEF web view = child `HWND` hosted in the tree.
- **Control / declarative-UI layer → pick one:**

| Path | Controls | Language | Notes |
|---|---|---|---|
| **`windows-reactor`** (recommended to try) | native WinUI 3 Fluent | **Rust** | React-style declarative (hooks, components, 55+ widgets, Composition + Animation Manager, theming, a11y), first-party (windows-rs team), ~3 MB binaries. **New (May 2026)** → coverage gaps possible, thin agent training data (feed agents its gallery/docs). |
| C# + WinUI 3 + XAML (fallback) | native WinUI 3 Fluent | C# | Mature, huge agent corpus, full tooling (XAML compiler, hot reload, Community Toolkit). Reintroduces a Rust↔C# FFI boundary. |
| raw `windows-rs` imperative WinUI | native but brutal | Rust | MS doesn't recommend WinUI-without-XAML for full apps. Avoid. |

- **Icons:** Segoe Fluent Icons via `FontIcon`/`SymbolIcon`.
- **Animations:** implicit animations, `ConnectedAnimation`, Composition API for the command-bar expand (reachable from both `windows-reactor` and C#).
- **Reference:** Edge's "Project Mai" is essentially this target (Mica, floating panels, rounded corners, vertical tabs, opaque address bar/active tab) - a good north star.

**Decision:** try `windows-reactor` first (keeps the whole app in Rust; see §18 monorepo payoff). De-risk it in the step-0 spike - a Fluent panel + the Cmd+T expand + CEF `HWND` embed. Fall back to C#/WinUI **only for the control layer** if it can't cover them; window/material/CEF stay `windows-rs` regardless.

**Starting Windows - the build-spec and what to carry over from macOS.**

- **The checklist is `parity.md`'s Win column.** Every `⬜` there is a to-do; the core rows are wired *by construction* (the capability already lives in the shared core), so the work is the native surface + binding, not re-implementing logic. Flip each cell as it lands, exactly like macOS did.
- **The de-risk spike is the first gate** (the Windows analogue of macOS's P0-1 focus-routing proof): a Fluent panel + the Cmd+T Composition expand + a CEF `HWND` embed, proving CEF↔WinUI **keyboard/focus routing** (§15 #1 - the highest risk on every frontend), window activation, and per-tab view lifecycle. Don't build UI on top until it holds. Replicate macOS's standing-regression discipline (`just focus-selftest` / `close-selftest` / `drag-selftest` / `redirect-selftest`) with in-process Windows equivalents - the CEF security-bump treadmill re-breaks these surfaces on every bump.
- **Codecs need a Windows build of their own.** The proprietary-codec CEF (H.264/AAC, ADR-0040/0042) is a *from-source* build; the current pipeline (`scripts/build-cef-codecs.sh`) targets **macOS-arm64 only**. Windows either gets its own codec-enabled CEF build or ships the **codec-less fallback** (the documented one-flag retreat). Also note the CEF web-platform gaps that bite Windows specifically - **WebGPU is broken out of the box on Windows** (a flag + two missing DLLs, rm §4.2).
- **Mirror the macOS structural decisions** so the two frontends stay one product: the **snapshot observation model** (§5, ADR-0005); AppKit-owns-frames → **the OS compositor owns the CEF view's frame** (SwiftUI/XAML structural churn around the platform view stalls Chromium's pipeline - CLAUDE.md gotcha, and the WinUI equivalent applies); the **custom in-sidebar window controls** idiom (traffic lights on macOS → caption buttons on Windows, ADR-0012 - a documented `style §2` divergence); and the shared **document-start injection substrate** (ADR-0017) that adblock-cosmetic, cookie-consent, and autofill all ride.

---

## 9. The Cmd+T command bar

Cross-platform behavior: global `Cmd/Ctrl+T` → a centered overlay (Spotlight position), text field focused, results list (open tabs, history, URL/search). Escape/click-away dismisses.

- **macOS:** centered `GlassEffectContainer` holding a glass search field + results, animated in with the interactive glass morph (`glassEffectID` transition) so it expands like Spotlight.
- **Windows:** centered Acrylic surface, expanded via `ConnectedAnimation`/Composition.

The bar queries the Rust core (`command_bar_query`) which unifies tabs/history/vault suggestions.

---

## 10. Password vault

**Do not build on Bitwarden's `sdk-internal`** - its license forbids using it to build non-Bitwarden apps. Two license-clean routes:

- **rbw** (doy) - mature, feature-complete unofficial Bitwarden client in Rust (auth, vault crypto, sync, agent). CLI+agent-shaped; vendor/adapt its client + crypto logic. MIT.
- **rwarden + rwarden_crypto** (nn1ks) - more library-shaped API client with crypto split out; dual MIT/Apache; less actively maintained.

Talk to bitwarden.com or a self-hosted **Vaultwarden**. Crypto per Bitwarden's security whitepaper (KDF PBKDF2/Argon2id → master key → stretched key → protected symmetric key; AES-256-CBC + HMAC-SHA256) using standard crates (`argon2`, `aes`, `hmac`).

Caveats: rbw doesn't support passkey/Duo as a **login** 2FA (TOTP/Yubico OTP fine); bitwarden.com flags CLI traffic as bot traffic, so device registration via personal API key is needed. Autofill surfaces through the core (`vault_lookup(host)`), consumed by the native UI.

---

## 11. Ad blocking

Native, no extension needed. Use the **`adblock`** crate (brave/adblock-rust - powers Brave, now also in Firefox).

- Hook it into the CEF **resource-request handler** for network blocking; inject cosmetic-filter CSS/scriptlets per page.
- Consumes uBlock Origin / EasyList / EasyPrivacy syntax directly (uBO-compatible resources).
- **Better than uBO Lite:** not bound by MV3 declarativeNetRequest caps - full cosmetic filtering + scriptlet injection, no rule-count limits.
- Disable `embedded-domain-resolver` when embedding to avoid binary bloat; enable single-thread perf features.
- **User-authored cosmetic rules share this path.** Per-site *Tweaks* - the Snip element remover, custom CSS, and userscripts (§20) - inject through the same document-start mechanism; filter-list rules and user Tweaks coexist in one injector, differing only in provenance.

---

## 12. Sync (separate service)

Not a browser feature - its own project. Auth, a sync server, conflict resolution, ideally E2E encryption. Sync tabs/spaces/settings. Treat as the highest-risk external dependency and build after the browser stands alone.

---

## 13. DevTools & automation

Both come from CEF (Chromium), independent of the UI.

- **DevTools:** full, real Chrome DevTools (all panels). `CefBrowserHost::ShowDevTools()` - popup or docked into a hosted view. If the Rust binding hasn't surfaced `show_dev_tools`, it's a one-line FFI call over `-sys`.
- **Automation:** enable `--remote-debugging-port` → full CDP. Same protocol as DevTools, so DevTools and web-automation are one surface.

---

## 14. Accessibility

A bonus of going native: the chrome gets platform accessibility **for free** - VoiceOver / NSAccessibility on macOS, Narrator / UI Automation on Windows - because you're using native controls. No AccessKit or custom a11y tree needed (which a Vello chrome would have required).

---

## 15. Risks - prototype these first

1. **CEF ↔ native embedding**: keyboard/focus routing between chrome and the embedded web view; window activation; per-tab view lifecycle.
2. **Cmd+T glass morph feel**: get the Spotlight-like expand right early - it's the signature interaction.
3. **Passkeys**: binding WebAuthn to the platform authenticator (Windows Hello / Touch ID) needs correct entitlements/config - validate in week one.
4. **Widevine/DRM**: only if you need protected video; requires a CEF build that includes it.
5. **Chromium update treadmill**: CEF tracks Chromium branches; budget recurring effort to bump CEF for security patches. This is the ongoing maintenance tax - plan for it.

---

## 16. Build order

Authoritative sequencing lives in `rm §5` (the five phases) and `mvp` (the macOS M1–M5 milestones); this is the coarse component-dependency chain they realize:

**Spike** (de-risk - done, see `spike.md`) → **Rust core** (`engine` + `tabs`/session model + `adblock` + CDP surface + `ffi`) → **macOS shell** (SwiftUI window, sidebar + vertical tabs + spaces, CEF via `NSViewRepresentable`, Cmd+T glass command bar) → **Vault** (rbw-derived client wired into the command bar + autofill) → **Sync service** (separate; last).

The chain is a *dependency* order, not a strict schedule. Everything hangs off the shared core, so once it stands, **the Windows shell (WinUI 3 mirror - Mica/Acrylic, Fluent, Segoe icons, Composition animations) is a parallel frontend track**, not a stage gated behind the vault: it depends only on the core + FFI (both done) and builds against `parity.md`'s Win column (§8, `rm §5.4`). macOS-only stages (vault polish, feature depth) and the Windows shell can proceed concurrently.

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
| Page customization | `tweaks` module - Snip / Restyle / Script over the adblock cosmetic path (§20) |
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
│  │  ├─ project.yml         # XcodeGen spec - SOURCE OF TRUTH (text, agent-edited)
│  │  ├─ Browser.xcodeproj   # GENERATED, disposable, gitignored
│  │  └─ Sources/            # SwiftUI
│  └─ windows/               # cargo crate (windows-reactor, native WinUI 3 in Rust)
│     └─ src/                # - no .sln/.csproj, no dotnet toolchain
│                            #   (fallback: C# .sln/.csproj + Sources/ if reactor falls short)
├─ bindings/                 # generated Swift (+ C# only if the C# fallback is used)
└─ scripts/                  # Makefile / justfile build orchestration
```

**Two languages, not three (if `windows-reactor` holds up):** the Windows app becomes an ordinary workspace crate built with `cargo build` - no separate C# project or dotnet toolchain - so agents drive it with the same commands as the core. Only the macOS app stays non-Rust (Liquid Glass's morph APIs are SwiftUI-only). C#/WinUI returns only as the Windows control-layer fallback.

**Xcode project handling (critical for agents):** never let agents hand-edit `.pbxproj` (opaque, merge-hostile). Use **XcodeGen** (`project.yml`) or **Tuist** to generate `.xcodeproj` from a plaintext spec. Agents edit the spec; a script regenerates the project. You initialize once (or generate); the durable artifact is text. SwiftPM is an alternative but Xcode projects handle app targets/signing/entitlements more smoothly.

**Headless CLI build paths (no IDE required):**
- Rust: `cargo build` / `cargo test`
- FFI: `cargo run --bin uniffi-bindgen` → regen `bindings/`
- macOS: `xcodegen generate` → `xcodebuild -scheme Browser build` (CLT, no GUI)
- Windows: `cargo build -p windows-app` (windows-reactor) - or `dotnet build Browser.sln` if using the C# fallback

Wrap in `scripts/` (`just build-macos`, `just build-windows`) so agents get pass/fail without an IDE. Constraint: `xcodebuild` is macOS-only, WinUI is Windows-only - full builds need both OSes or per-OS CI runners; each platform's agent has a complete headless path.

---

## 19. Agent layer (future)

Lives in `crates/agent`, over the same core.

- **ACP (Agent Client Protocol):** host local agents behind a standardized client↔agent boundary; the SwiftUI/WinUI chat panel speaks ACP so you're not locked to one agent backend.
- **Browser-as-tool:** CEF + CDP (§13) already provides navigate / extract / download / screenshot - expose as an MCP/ACP tool. "Pull in sites via the real browser" is near-free.
- **Virtual folder:** a sandboxed working dir the agent reads/writes; downloads route here, artifacts for the user are produced here.
- **Permission model:** one consent surface, escalating scopes - virtual folder → named paths → shell exec → full system. The in-sandbox vs. real-computer boundary is a hard, auditable line; that boundary is the entire safety story for full-computer access.
- **Tweaks as an agent surface:** the per-site content-modification store (§20) is declarative and inspectable, so the agent can read, propose, and apply *Tweaks* - e.g. author a Snip to hide a cookie banner on request. Human-written userscripts and agent-authored Tweaks are the *same object* through the same store; agent-authored Script Tweaks pass the same consent gate as any other (§20).
- **Commands as an agent surface (Phase 4):** the Commands registry (§21, ADR-0034) is the chrome-automation counterpart - the agent can read, propose, create, and edit Commands in-app (natural-language → TypeScript, iterate in the settings editor). Agent-authored Commands are the same object as hand-written ones and pass the same consent gate; execution stays human-initiated (command bar / palette / bound shortcut), not autonomous agent-mode.

---

## 20. Page customization - Tweaks (Snip / Restyle / Script)

**What it is.** Per-site page customization: the native answer to Arc Boosts, uBlock Origin's element picker, Stylus, and Tampermonkey - unified into one feature. A **Tweak** is a saved, URL-scoped set of changes applied to matching pages. This is the concrete anchor of the "extensible by scripting and automation" goal (§19): the object a power user hand-writes is the same object an agent authors and applies.

**One primitive, three tiers.** All three are the *same* underlying thing - URL-scoped content injected at document-start - surfaced at rising levels of power:

| Tier | Name | What the user does | What it compiles to |
|---|---|---|---|
| 1 | **Snip** | Click an element to remove it (visual picker, no code) | An element-hiding rule (`selector { display:none !important }`) |
| 2 | **Restyle** | Write custom CSS for the site | A scoped stylesheet |
| 3 | **Script** | Write / inject custom JS (power users) | A userscript in an isolated world |

Snip covers the ~90% case (kill a banner, a sticky header, a paywall veil) with zero code; Restyle and Script are the power-user escape hatches. One scoping model and one injection path serve all three - build the primitive once, and all three tiers come nearly free (the roadmap's recurring "one primitive, many audiences" leverage, rm §5.3).

**Shared injection path with adblock (§11).** A Snip is mechanically identical to an adblock *cosmetic* rule - element-hiding CSS injected per page - and the Script tier is the same scriptlet-injection path adblock already uses. Tweaks therefore **reuse the engine's document-start injector** rather than adding a parallel mechanism; the only difference from §11 is *provenance* (user-authored, per-site, editable in the UI) versus filter-list-sourced. Filter-list rules and user Tweaks coexist in one injector.

**Core module (`tweaks`).** Owns the rule store, the URL-scoping / match logic, editor state, and import/export. Frontend-agnostic like the rest of the core (§3); the native layer is only the picker overlay + editor + manager bound to it - its UI treatment is specified in the UI style guide. It emits injections through the same `engine` hook `adblock` feeds.

**Scoping.** One match-pattern grammar across all three tiers. Default scope for a new Tweak is the current site (registrable domain); power users edit a match pattern (`*://*.example.com/path*`, à la WebExtension match patterns / Tampermonkey `@match`). Scoping lives in the core, so it is identical on every platform.

**Injection mechanics.**
- **CSS (Snip + Restyle)** - injected at **document-start** to avoid a flash of the un-hidden / un-styled element (FOUC), and re-asserted for dynamically-inserted DOM.
- **JS (Script)** - injected via CDP `Page.addScriptToEvaluateOnNewDocument` into an **isolated world** (a separate V8 context) at document-start by default, with an opt-in `run-at` of document-end / idle. The research (rm §3.1) confirms this path is production-grade. A Tampermonkey-style metadata header (`@match`, `@run-at`, name, version) is the script's manifest.

**Sync & portability.** A Tweak is a portable document (match pattern + rules / CSS / JS + metadata). Per rm §3.4 the model is **sync-ready from the start** - Tweaks are exactly the per-device user data sync must carry - even though sync itself ships later. Tweaks export / import as a self-contained file for sharing.

**Safety - the load-bearing part.** A shared Tweak carrying JS is arbitrary code execution in the user's authenticated session - the userscript supply-chain risk, and a sibling of the AI prompt-injection concern (rm §8.4). The engine treats it accordingly:
- **No silent auto-run.** Importing a Tweak never enables it implicitly; enabling is an explicit user act.
- **Code is always shown, never obfuscated-run.** The full CSS / JS is visible and diffable before enable, and on any change to a Script Tweak (a modified script re-prompts).
- **JS requires explicit, worded consent** on enable; Snip / Restyle (no DOM or script access) are lower-risk and lighter-gated - though CSS can still exfiltrate via resource requests (e.g. `background-image`), noted rather than ignored.
- **Isolation is not a sandbox against the user's own session.** The isolated world stops the *page* from clobbering / detecting the script; it is not a security boundary against the script itself. The consent gate, not isolation, is the real control. Per-site enable / disable, a global kill switch, and an on-page indicator when any Tweak is active are all required.

**FFI surface.** Coarse-grained like the rest (§5): `tweaks_for_host(host)`, `snip_begin` / `snip_commit(selector)`, `tweak_upsert`, `tweak_toggle`, `tweak_import` / `tweak_export`, plus a `TweakActivated` event so the UI can show the active-Tweak indicator. Names follow the shared vocabulary (`tweak`, `snip`) per the UI style-guide terminology contract.

---

## 21. Browser automation - Commands

**What it is.** User-defined browser-chrome automation - the Glide-class answer to "hack your browser like
your editor." A **Command** is a saved TypeScript module that runs against a typed `CommandContext` API
(tabs, spaces, navigate, process spawn, paths, clipboard, notify). Distinct from **Tweaks** (§20, page
injection) and the external **CDP** port (§13, dev/agent tooling). ADR-0034.

**Invocation.** Primary: the **⌘T command bar** - Commands rank alongside URLs, open tabs, history, and
search in one unified list. Secondary (Phase 3): a **⌘K command palette** biased toward actions (built-ins +
the same registry; no URL/search rows). Settings: a **Commands** manager section (list, editor, cookbook,
inline test).

**Core module (`commands`).** Registry, per-file store (`commands/`), embedded TS runtime, permission checks
(`process.exec` tier aligned with §19). Agent-authored Commands are the same object as hand-written ones.

**Agentic authoring (Phase 4).** Future AI feature: in-app create/edit of Commands via the agent layer (§19) -
describe a workflow, review generated TypeScript in the manager, enable with the same gates. V1 ships manual
editing + stubs/cookbook (P3-15/P3-16); agentic authoring is optional and later.

**Safety.** User-owned config; code shown before enable, re-prompt on change, global kill switch. Not an
extension sandbox - the user is responsible, with explicit gates on the dangerous primitives.

**FFI surface (illustrative):** `commands_query`, `command_run`, `command_upsert`, `command_toggle`,
`command_import` / `command_export`.
