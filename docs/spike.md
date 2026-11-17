# CEF Spike

**Run:** 2026-04-14 · **Outcome:** GO.

De-risking spike for the CEF foundation before MVP1. It has since been acted on: the workspace compiles,
`bundle.rs` lays out the `.app`, and a single CEF tab renders real sites (see `CLAUDE.md` for how to run).
This doc is kept as the durable **engine reference** - the crate choice, the hosting model, and the
passkey/adblock notes the spike produced. The point-in-time "missing files / build blockers" the spike
originally inventoried are all resolved and have been dropped; where a recommendation was later superseded,
it's noted below.

Cross-refs: `spec §N` = architecture.md · `rm §N` = roadmap.md.

---

## 1. Engine crate: `cef` (tauri-apps/cef-rs) - confirmed

`cef` + `cef-dll-sys` `149.3.0+149.0.6` → **Chromium 149** (full WebAuthn L3, DevTools, all Chromium
features). Chosen and still in place (`Cargo.toml`). Why:

- **Already integrated and ergonomic.** The high-level `wrap_client!` / `wrap_*_handler!` /
  `wrap_window_delegate!` / `wrap_task!` macros generate the CEF trait impls, so handlers are written in
  idiomatic Rust with no hand-written C FFI shims - critical for agent-editability (`client.rs`, `app.rs`).
- **Prebuilt binaries, no Chromium compile.** `cef-dll-sys` downloads the prebuilt CEF distribution and
  builds only `libcef_dll_wrapper` via cmake at compile time (needs `cmake` + `ninja`). `api_hash(...)` is
  checked at load to catch ABI mismatch.
- **No viable alternative.** The `cef-ui` (hytopiagg) crate the spec once listed was not found on crates.io;
  `wef` / `bevy_cef` / `fenestra-cef` are unsuitable. `cef` is the de-facto standard Rust binding.

**Superseded recommendation:** the spike proposed a `build.rs` to assemble the framework + helper bundle.
The project instead uses a dedicated **`bin/bundle.rs`** (via `cef::build_util::mac::build_bundle`) that lays
out `target/bundle/browser.app` - there is deliberately **no `build.rs`**. See `CLAUDE.md`.

---

## 2. Hosting model: standalone (Views) vs. embedded (Option B)

Two paths, both proven in code:

- **Standalone / Views** (`lib.rs`) - CEF's Views framework creates and owns its own `NSWindow`. What the
  `browser` binary uses today; the simplest way to get pixels on screen.
- **Embedded / "Option B"** (`embed.rs`) - the native app owns the `NSWindow` and hands CEF a child `NSView`
  (an ALLOY / chromeless browser) via `browser_host_create_browser` + `WindowInfo`. **This is the path the
  SwiftUI shell needs** - SwiftUI hosts the CEF `NSView` through `NSViewRepresentable` so the native chrome
  (Liquid Glass sidebar, Cmd+T) owns the window. The `runtime_style` field (DEFAULT vs. ALLOY) controls
  whether CEF draws its own chrome.

The macOS platform requirements (framework loading via `LibraryLoader`; a CEF-conformant `NSApplication`
subclass; an absolute `root_cache_path`) live in `mac.rs` - see `spec §7` and `CLAUDE.md`.

---

## 3. Focus / keyboard routing - the carried-forward risk

The highest-risk item (`spec §15` #1) and still the one to prove: keyboard/focus routing between the embedded
CEF `NSView` and SwiftUI - typing in a page vs. Cmd+T vs. sidebar focus, and click-to-focus transitions. The
Views path creates its own window, so it can't answer this; it must be validated on the embedded path in
`mvp` M2. Tracked as backlog **P0-1**. Everything built on the shell depends on it - resolve before building
UI on top.

---

## 4. Passkeys - signed entitlements, then Chromium's platform authenticator

Passkeys work through Chromium's own authenticator stack (CEF passes WebAuthn to
the macOS platform authenticator / iCloud Keychain). There is no custom WebAuthn
protocol in our core. What they require is bundle correctness (ADR-0061):

- **A signed, entitled bundle.** iCloud Keychain passkeys for arbitrary sites
  need Apple's `com.apple.developer.web-browser` and
  `com.apple.developer.web-browser.public-key-credential` entitlements (request-
  gated, `rm §6.1`, form:
  https://developer.apple.com/contact/request/macos-browsers-passkeys/) plus
  `keychain-access-groups` matching Chromium's branding group
  (`.org.chromium.Chromium.webauthn` / `TEAMID.org.chromium.Chromium.webauthn`).
  `just build-release` re-signs inside-out with Developer ID when a certificate
  is present (`scripts/codesign-macos-app.sh`). The WebAuthn JS API turns on
  only when the running binary actually carries the passkey entitlement.
- **Unentitled builds must not enter the ceremony.** Missing those entitlements
  used to SIGABRT (CRASH-0003). The browser process disables Blink
  `WebAuthentication` unless `SecTaskCopyValueForEntitlement` sees the
  capability.
- **Correct helper-bundle structure.** `browser.app/Contents/{MacOS,Frameworks,Helpers}`
  with per-role helper `.app`s - assembled by `bundle.rs`.
- **Vault passkeys** are a later second authenticator (P2-50), not this path.

Tracked as backlog **P0-2** (blocked on Apple granting the macos-browsers-passkeys entitlement).

---

## 5. Adblock - the request-path hook

The `adblock` crate (brave/adblock-rust) is proven (it powers Brave); the only open question the spike
flagged is whether the CEF request-interception hook is reachable through the `cef` binding. Plan (`spec §11`):

- Implement a **`ResourceRequestHandler`** (or `RequestContextHandler`, likely the better per-tab hook) on the
  client, check each request URL against the loaded `Engine`, and cancel matches on the request path.
- Inject **cosmetic filters** (CSS/scriptlets) per page at document-start - the same injector `tweaks` reuses
  (`spec §20`). Load EasyList / EasyPrivacy / uBO-compatible lists at startup; disable
  `embedded-domain-resolver` to avoid binary bloat.
- If `request_handler` isn't surfaced by the high-level `cef` crate, it's a manual FFI call over
  `cef-dll-sys` - the same fallback shape as any un-surfaced CEF API.

Tracked as backlog **P0-3**.

---

## 6. Outcome & carried-forward risk

**GO** - the architecture and CEF patterns were sound, and the prerequisites the spike identified (the entry
points, the `mac` module, resource loading, the bundle layout) have all been built. No redesign was needed.

Risks the spike explicitly carried forward, still live:

- **CEF ↔ SwiftUI focus/keyboard routing** (§3 above) - de-risk on the embedded path in `mvp` M2 / P0-1.
- **`cef` crate API coverage** - if `ShowDevTools` / `ResourceRequestHandler` aren't surfaced, drop to manual
  FFI over `cef-dll-sys`. Expected and manageable. (DevTools is already wired via
  `browser_toggle_dev_tools` + a context-menu Inspect Element.)
- **macOS bundle exactness** - the helper-process bundle structure must be precise for CEF to run; `bundle.rs`
  owns it.
