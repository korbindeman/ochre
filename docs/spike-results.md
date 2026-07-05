# Step 0 — CEF Spike Results

**Status:** DISCOVERY (knowledge deliverable, no production code changed)
**Date:** 2026-04-14
**Spike goal:** De-risk the foundation before MVP1 — confirm a single CEF tab renders in a native macOS `NSView`, DevTools opens, a passkey login works, and adblock blocks on the request path.

---

## 1. CEF Binding Crate: Choice and Rationale

### Candidates evaluated

| Crate | Source | Version (spike) | Notes |
|---|---|---|---|
| **`cef` (tauri-apps/cef-rs)** | crates.io | `149.3.0+149.0.6` | The workspace already depends on this. `cef-dll-sys` downloads prebuilt CEF binary distribution and builds `libcef_dll_wrapper` via cmake. Provides high-level `wrap_*!` macros that generate trait implementations for Client, DisplayHandler, LifeSpanHandler, LoadHandler, Task, App, BrowserProcessHandler, WindowDelegate, BrowserViewDelegate, etc. |
| `cef-ui` (hytopiagg) | crates.io (searched) | — | Listed in architecture spec §4 as an alternative, but no crate published under `cef-ui` or `cef_ui` was found on crates.io as of this spike. May exist under a different name or be GitHub-only. |
| `wef` | crates.io | 0.6.0 | Higher-level CEF wrapper for embedding webview functionality; opinionated, less direct access to CEF APIs. |
| `bevy_cef` / `bevy_cef_core` | crates.io | 0.11.0 | Bevy-game-engine-specific CEF integration; not suitable for a browser app. |
| `fenestra-cef` | crates.io | 0.1.0 | Cross-platform webview runtime; early, thin. |

### Decision: `cef` (tauri-apps/cef-rs) — **confirmed**

**Why:**

1. **Already integrated.** The workspace `Cargo.toml` declares `cef = "149.3.0"` and `cef-dll-sys = "149.3.0"` as workspace dependencies. `crates/core/Cargo.toml` depends on `cef` with the `sandbox` and `build-util` features. The code in `lib.rs`, `app.rs`, and `client.rs` already uses the `cef` crate's types and `wrap_*!` macros extensively. No migration cost.

2. **High-level, ergonomic API.** The `wrap_client!`, `wrap_display_handler!`, `wrap_life_span_handler!`, `wrap_load_handler!`, `wrap_app!`, `wrap_browser_process_handler!`, `wrap_window_delegate!`, `wrap_browser_view_delegate!`, and `wrap_task!` macros let us implement CEF handler traits in idiomatic Rust without manually writing C FFI shims. This is critical for agent-editability — the generated code is clean and readable.

3. **Prebuilt binary distribution.** `cef-dll-sys` downloads prebuilt CEF binary distributions (Spotify cefbuilds) and builds only `libcef_dll_wrapper` via cmake. No full Chromium compilation. cmake (3.x) and ninja are available in the toolchain. The `build-util` feature provides the `cef` crate's build utilities for bundling.

4. **API version hash verification.** `cef::api_hash(sys::CEF_API_VERSION_LAST, 0)` is called in `load_cef()` to verify the loaded CEF framework matches the compiled API version — a safety mechanism against ABI mismatch.

5. **No viable alternative found.** The `cef-ui` (hytopiagg) alternative mentioned in the spec was not found on crates.io. The `cef` crate is the de facto standard Rust CEF binding, maintained by the tauri-apps organization.

**Chromium version:** CEF 149.3.0+149.0.6 → Chromium 149. This is a recent stable branch with passkey/WebAuthn, DevTools, and all Chromium features available.

---

## 2. What Worked

### 2a. Workspace structure and crate layout
- The Cargo workspace compiles (modulo missing binary entry points — see §3 below). The two workspace members (`crates/core` and `crates/ffi`) are correctly configured.
- `browser-core` exposes `cdylib` + `rlib` crate types for FFI consumption.
- `browser-ffi` re-exports `browser_core` and surfaces `core_version()`.
- The module structure (`app`, `client`, `mac`, `resources`) follows the architecture spec's module map.

### 2b. CEF initialization path
- `lib.rs::run()` implements the full CEF lifecycle: library loading → API hash verification → `setup_simple_application` → `execute_process` (browser vs. helper process dispatch) → `initialize` → `run_message_loop` → `shutdown`.
- Browser/helper process detection via the `--type` command-line switch (standard CEF pattern). Helper processes return after `execute_process`; the browser process continues to `initialize`.
- The `Settings` struct uses `no_sandbox` based on the `sandbox` feature flag, which is enabled by default.

### 2c. App and handler implementations
- `BrowserApp` (in `app.rs`) implements the CEF `App` trait with a `BrowserProcessHandler` that fires `on_context_initialized` to create the first browser instance.
- Two rendering paths are supported:
  - **Views framework** (default): `browser_view_create` + `window_create_top_level` with `BrowserWindowDelegate` and `BrowserBrowserViewDelegate`. This creates a native window with a browser view — the windowed rendering approach from §4.
  - **Native popup** (`--use-native` switch): `browser_host_create_browser` with `WindowInfo`. This is the older path, useful for debugging.
- Runtime style (DEFAULT vs. ALLOY) is controlled via `--use-alloy-style` switch. ALLOY enables custom error pages and platform-specific title handling.
- `BrowserClient` wires up Display, LifeSpan, and Load handlers with proper shared state via `Arc<Mutex<BrowserHandler>>`.
- `BrowserHandler` manages the browser list, closing state, and thread-safe task posting (UI thread dispatch via `post_task`).

### 2d. macOS application setup
- The architecture spec calls for NSApplication subclassing implementing CEF protocols. The `mac` module (referenced as `pub mod mac` and `mod mac` in `client.rs`) provides `load_cef_framework`, `setup_simple_application`, and `setup_app_delegate` functions. These are declared but the module file (`crates/core/src/mac.rs` or `crates/core/src/mac/`) is **not yet present on disk** — see §3.
- `objc2` and `objc2-app-kit` dependencies are configured with the needed features (`NSApplication`, `NSEvent`, `NSResponder`, `NSUserInterfaceValidation`, `NSView`, `NSWindow`).

### 2e. Window and view delegates (Views framework)
- `BrowserWindowDelegate` implements `ViewDelegate` (preferred size 1024×768), `PanelDelegate`, and `WindowDelegate` (on_window_created, on_window_destroyed, can_close, initial_show_state, window_runtime_style). This is the standard CEF Views framework pattern for creating native top-level windows.
- `BrowserBrowserViewDelegate` implements popup browser view creation and browser/panel runtime style. The `on_popup_browser_view_created` handler creates a new top-level window for popups (including DevTools — the `_is_devtools` parameter is present though not yet gated on).
- The view delegate approach gives us native NSView hosting for free — the CEF browser view is a native NSView that SwiftUI can embed via `NSViewRepresentable`.

### 2f. Error handling and graceful shutdown
- `on_load_error` renders a custom HTML error page (ALLOY style only) with the failed URL, error text, and error code. `ERR_ABORTED` is skipped (expected for navigations).
- `do_close` / `on_before_close` / `close_all_browsers` implement graceful shutdown: track closing state, remove browsers from the list on close, and call `quit_message_loop` when all browsers are gone.
- Task-based thread dispatch (`ShowMainWindowTask`, `CloseAllBrowsersTask`) ensures UI operations happen on the UI thread.

### 2g. macOS SwiftUI placeholder
- `apps/macos` has a working XcodeGen `project.yml` with a placeholder SwiftUI app (`BrowserApp.swift`) showing the slim-chrome layout: minimal toolbar (back/forward/reload + sidebar toggle), hidden title bar, content region placeholder for CEF NSView. The `NSViewRepresentable` integration point is clearly marked.
- The justfile has `gen-macos` and `build-macos` recipes for headless builds via `xcodegen generate` + `xcodebuild`.

---

## 3. What Blocked / Is Incomplete

### 3a. Missing binary entry points (BUILD BLOCKER — critical)
`crates/core/Cargo.toml` declares two binary targets:
```toml
[[bin]]
name = "browser"
path = "src/bin/browser.rs"

[[bin]]
name = "browser_helper"
path = "src/bin/browser_helper.rs"
```
Neither `src/bin/browser.rs` nor `src/bin/browser_helper.rs` exist on disk. `cargo build --workspace` fails with:
```
error: can't find bin `browser` at path `.../crates/core/src/bin/browser.rs`
error: can't find bin `browser_helper` at path `.../crates/core/src/bin/browser_helper.rs`
```
**Impact:** The workspace does not compile. This must be fixed before any CEF binary can be built or run. The browser binary should call `browser_core::run()`; the helper binary should call `execute_process` and return (it's the CEF helper process entry point for render/GPU/plugin subprocesses).

**This is the primary blocker preventing the spike from being verified at runtime.** The Rust code for CEF initialization, app handlers, and client handlers is complete, but without the binary entry points, nothing can actually execute.

### 3b. Missing `mac` module (BUILD BLOCKER — critical)
`lib.rs` declares `pub mod mac;` and `client.rs` declares `mod mac;`, but no file exists at `crates/core/src/mac.rs` or `crates/core/src/mac/mod.rs`. The code references:
- `mac::load_cef_framework()` → returns `mac::LibraryLoader`
- `mac::setup_simple_application()` → sets up NSApplication
- `mac::setup_app_delegate()` → returns an app delegate
- `mac::platform_show_window()` — platform-specific window showing (ALLOY style)
- `mac::platform_title_change()` — platform-specific title change (ALLOY style)

**Impact:** Even if the binary entry points existed, the crate would not compile because the `mac` module is missing. This module must implement Objective-C interop (via `objc2`/`objc2-app-kit`) for:
- Loading the CEF framework from the app bundle's `Frameworks` directory
- Subclassing NSApplication (CEF requires `NSApplication` to implement certain protocols)
- Setting up an app delegate that loads `MainMenu.nib` and configures app menus
- Platform-specific window showing and title changes for the ALLOY runtime style

### 3c. Missing `resources` module
`lib.rs` declares `pub mod resources;` but no file exists. This module is expected to handle resource loading (the `[package.metadata.cef.bundle]` in `Cargo.toml` references `resources_path = "resources"`). Likely contains or will contain embedded HTML/CSS/JS resources for error pages or internal pages. Not a build blocker if `resources` is only referenced from `lib.rs`'s `pub mod` declaration (Rust will error on a missing module file).

### 3d. Missing `build.rs`
`crates/core` has no `build.rs`. CEF requires a build script to:
- Download the prebuilt CEF binary distribution (or verify it's present)
- Build `libcef_dll_wrapper` via cmake
- Copy CEF framework and resources into the output directory
- Set up the helper process bundle structure

The `cef` crate's `build-util` feature provides build utilities, but a `build.rs` is needed to invoke them. The `[package.metadata.cef.bundle]` metadata (`helper_name = "browser_helper"`, `resources_path = "resources"`) suggests this is planned but not yet implemented.

### 3e. CEF binary distribution not downloaded
No CEF binary distribution was found in the workspace. The `cef-dll-sys` crate's build process typically downloads this, but since `build.rs` is absent (or not yet triggering the sys crate's build), the CEF framework (`Chromium Embedded Framework.framework`) is not present. Without it, the app cannot link or run.

### 3f. No adblock integration yet
The architecture spec (§11) calls for the `adblock` crate (brave/adblock-rust) hooked into the CEF resource-request handler. No `adblock` dependency is declared in any `Cargo.toml`, and no `ResourceRequestHandler` or `RequestContextHandler` implementation exists in `client.rs`. The CEF `Client` trait in `client.rs` only implements `display_handler`, `life_span_handler`, and `load_handler` — no `request_handler`. This is expected for the spike stage but must be prototyped to confirm adblock works on the request path.

### 3g. No DevTools API surface
`CefBrowserHost::ShowDevTools()` is not called anywhere in the codebase. The `BrowserBrowserViewDelegate::on_popup_browser_view_created` handler receives an `_is_devtools` parameter but doesn't gate on it. DevTools should be reachable via `browser.host().show_dev_tools(...)` or equivalent — needs a `RequestContextHandler` or a menu item that calls it. This is a one-line FFI call if the `cef` crate surfaces it; if not, it's a manual FFI call over `-sys`.

### 3h. No passkey/WebAuthn configuration
No entitlements or configuration for WebAuthn/passkey support is present. On macOS, passkeys require:
- The app to be signed (or at least have a valid entitlements plist) for the platform authenticator (Touch ID / Security Keys) to work.
- The CEF `Info.plist` to include `NSAppleEventsUsageDescription` or equivalent entries.
- The helper process bundle to have the correct entitlements.

The `project.yml` sets `CODE_SIGNING_REQUIRED: NO` and `CODE_SIGNING_ALLOWED: NO` (for headless CI builds), which means passkey/WebAuthn cannot be tested in this configuration. Testing passkeys requires a signed app bundle with the proper entitlements — this is a configuration gap, not a code gap.

### 3i. Focus/keyboard routing not yet prototyped
The architecture spec (§15 risk #1, MVP risk #1) identifies keyboard/focus routing between the native chrome and the embedded CEF NSView as the highest-risk item. No SwiftUI ↔ CEF focus integration exists yet — the macOS app is a placeholder with no actual `NSViewRepresentable` wrapping the CEF view. This needs to be prototyped in M2 per the MVP plan, but the spike should at minimum confirm that the CEF browser view receives keyboard focus when clicked.

---

## 4. Focus / Routing Issues Found

No runtime testing was possible (due to the build blockers in §3a–§3b), but static analysis reveals:

1. **Views framework vs. native popup path.** The code supports both paths. The Views framework (`browser_view_create` + `window_create_top_level`) is the default and is the correct choice for MVP — it provides native NSView embedding via the view hierarchy. The native popup path (`--use-native`) is a fallback for debugging.

2. **Thread dispatch pattern.** `BrowserHandler::show_main_window` and `close_all_browsers` correctly check `currently_on(ThreadId::UI)` and post tasks to the UI thread if not on it. This is the correct CEF pattern — all browser/host operations must happen on the UI thread. This will need to be respected by the FFI layer when Swift calls into the core.

3. **No `NSViewRepresentable` yet.** The CEF browser view is created via the Views framework, which creates its own `NSWindow`. For the MVP architecture (SwiftUI hosts the CEF view in a content region), we need to extract the `NSView` from the browser view and embed it in a SwiftUI `NSViewRepresentable`, rather than letting CEF create its own window. This is a known architectural decision that must be made in M2 — either:
   - **Option A:** Use the Views framework's window (CEF owns the window; SwiftUI adds chrome as subviews). Simpler but less native feel.
   - **Option B:** Use `browser_host_create_browser` with a parent `NSView` that SwiftUI provides via `NSViewRepresentable`. More work but gives SwiftUI full control over window chrome — the correct path for the Liquid Glass design.

   The `--use-native` path in `app.rs` already uses `browser_host_create_browser` with `WindowInfo`, which is the foundation for Option B. The `runtime_style` field on `WindowInfo` (DEFAULT vs. ALLOY) controls whether CEF draws its own chrome.

4. **Keyboard focus.** Not testable yet. The key question for M2: when the user clicks into the CEF web view, does the SwiftUI chrome lose focus correctly? And when the user presses Cmd+T, does focus return to the SwiftUI command bar? This requires `NSResponder` chain management — making the CEF `NSView` part of the SwiftUI responder chain, or using a global key event monitor.

---

## 5. Passkey Configuration Notes

WebAuthn/passkeys in CEF on macOS require:

1. **App signing.** The platform authenticator (Touch ID / Security Keys) is only available to signed apps with the correct entitlements. The current `project.yml` disables code signing (`CODE_SIGNING_REQUIRED: NO`, `CODE_SIGNING_ALLOWED: NO`). To test passkeys:
   - Enable code signing (at minimum ad-hoc: `CODE_SIGN_IDENTITY: "-"` is already set, but `CODE_SIGNING_ALLOWED: NO` blocks it).
   - Add an entitlements plist with `com.apple.security.cs.allow-unsigned-executable-memory` (CEF needs this) and potentially `com.apple.developer.webauthn` (if Apple requires it — needs verification).
   - The helper process bundles (render, GPU, plugin) must also be signed with the same identity.

2. **CEF helper process structure.** CEF on macOS requires a specific bundle structure:
   ```
   Browser.app/
   ├─ Contents/
   │  ├─ MacOS/
   │  │  ├─ browser           ← main process
   │  │  └─ browser_helper     ← helper (renamed per Cargo.toml metadata)
   │  ├─ Frameworks/
   │  │  └─ Chromium Embedded Framework.framework/
   │  └─ Helpers/
   │     ├─ browser_helper.app/   ← helper bundle for render/GPU/plugin
   │     │  └─ Contents/
   │     │     └─ MacOS/
   │     │        └─ browser_helper
   │     └─ ...
   ```
   The `[package.metadata.cef.bundle]` in `Cargo.toml` (`helper_name = "browser_helper"`, `resources_path = "resources"`) suggests this structure is planned. The `build.rs` must set it up.

3. **Info.plist requirements.** The CEF `Info.plist` needs `LSMinimumSystemVersion`, and potentially `NSAppleEventsUsageDescription` for some WebAuthn flows. The `project.yml` sets `INFOPLIST_KEY_LSMinimumSystemVersion: "14.0"` which is sufficient for the OS but WebAuthn-specific plist entries may be needed.

4. **Chromium version.** CEF 149 / Chromium 149 includes full WebAuthn Level 3 support (passkeys, conditional UI / autofill-mediated WebAuthn). This is sufficient — no Chromium version concern.

5. **Key architectural note:** Passkeys in CEF use the macOS platform authenticator through Chromium's internal `Authenticator` implementation. CEF passes WebAuthn requests through to the OS — there's no custom Rust code needed for passkeys to work, just correct bundle structure and entitlements. The "passkey login works" spike test is fundamentally a build/config validation, not a code implementation test.

---

## 6. Adblock Integration Notes

The spike's adblock goal (confirm `adblock` blocks on the request path) has not been prototyped yet. What's needed:

1. **Add `adblock` dependency.** Add `adblock = { version = "0.x" }` (brave/adblock-rust, published on crates.io) to `crates/core/Cargo.toml` and possibly the workspace dependencies.

2. **Implement `ResourceRequestHandler`.** In `client.rs`, the `BrowserClient` must implement `request_handler()` returning a `ResourceRequestHandler` that:
   - Intercepts `on_before_browse` or `get_resource_request_handler` / `on_before_resource_request`.
   - Checks the request URL against the loaded filter list.
   - Returns `RV_CANCEL` to block matching requests.

3. **Cosmetic filtering.** Inject CSS/scriptlets per page via `OnLoadComplete` or a `DisplayHandler::on_console_message`-style hook. The `adblock` crate's `cosmetic_filter` API provides the CSS rules and scriptlets to inject.

4. **Filter list loading.** EasyList / EasyPrivacy / uBlock Origin-compatible lists are loaded at startup. The `adblock` crate's `Engine` struct handles parsing and matching. Disable `embedded-domain-resolver` to avoid binary bloat (§11).

5. **Request context.** CEF's `RequestContext` and `RequestContextHandler` may be the better hook point than `Client::request_handler` for per-tab adblock. The `cef` crate's API needs to be checked for whether it surfaces `RequestContextHandler`.

6. **Expected outcome:** The `adblock` crate is proven (powers Brave's adblock). The risk is not whether the crate works but whether the CEF request interception hook is accessible through the `cef` Rust binding. If `request_handler` or `get_resource_request_handler` is not surfaced by the `cef` crate, it's a manual FFI call over `cef-dll-sys` — similar to the DevTools `ShowDevTools` situation.

---

## 7. Go / No-Go Recommendation

### GO — proceed to MVP1 milestones, with prerequisites

The CEF spike is **structurally validated** but **not yet runtime-verified** due to missing code artifacts. The architecture and code patterns are sound; the blocking issues are well-understood and have clear solutions.

**Prerequisites to clear before M1 work begins** (in order):

1. **Create `crates/core/src/bin/browser.rs`** — the main process entry point. Calls `browser_core::run()`. ~10 lines.

2. **Create `crates/core/src/bin/browser_helper.rs`** — the helper process entry point. Calls `execute_process` and returns the exit code. ~15 lines.

3. **Create `crates/core/src/mac.rs`** (or `mac/mod.rs`) — the macOS platform module. Implements `LibraryLoader`, `load_cef_framework()`, `setup_simple_application()`, `setup_app_delegate()`, and the platform-specific `platform_show_window` / `platform_title_change` functions using `objc2` / `objc2-app-kit`. This is the most substantial missing piece (~150–300 lines of unsafe Objective-C interop).

4. **Create `crates/core/src/resources.rs`** — the resources module (even if initially empty or minimal, to satisfy the `pub mod resources` declaration).

5. **Create `crates/core/build.rs`** — the build script that invokes CEF build utilities (from the `build-util` feature) to download/build the CEF binary distribution and set up the framework + helper bundle structure.

6. **Verify `cargo build --workspace` compiles.** Then attempt a runtime launch of the `browser` binary to confirm CEF initializes and renders a page.

**Only after these prerequisites are cleared:**
- Confirm DevTools opens (`host.show_dev_tools()` or equivalent).
- Confirm a passkey login works (with a properly signed app bundle).
- Prototype adblock on the request path (`adblock` crate + `ResourceRequestHandler`).

**Rationale for GO despite incomplete runtime verification:**
- The CEF binding crate (`cef` / `cef-dll-sys`) is correctly chosen and integrated. No alternative offers a better path.
- The CEF initialization, app, client, handler, and delegate code follows correct CEF patterns (verified against CEF sample app conventions).
- The Views framework path provides native NSView embedding — the architectural foundation for MVP's SwiftUI hosting.
- All blocking issues are code-completion tasks, not architectural dead ends. No redesign needed.
- The architecture spec and MVP plan are sound; the spike has de-risked the *approach* if not yet the *runtime*.

**Risk carried forward:**
- **CEF ↔ SwiftUI focus/keyboard routing** (highest risk per §15) — not yet prototyped. M2 is the right milestone to de-risk this, and the spike doesn't need to resolve it.
- **`cef` crate API coverage** — if `ShowDevTools` or `ResourceRequestHandler` are not surfaced by the high-level `cef` crate, manual FFI over `cef-dll-sys` will be needed. This is expected and manageable (the `-sys` crate provides raw C bindings).
- **macOS bundle structure** — the helper process bundle structure must be exactly right for CEF to function. The `build.rs` must handle this correctly. This is the most fiddly part of CEF on macOS.

---

## 8. Summary of Findings for Future Runs

| Spike Question | Answer |
|---|---|
| Which CEF binding crate? | **`cef` (tauri-apps/cef-rs) v149.3.0+149.0.6** — already integrated, ergonomic `wrap_*!` macros, prebuilt binaries, no viable alternative found |
| Single CEF tab in native NSView? | **Code exists** (Views framework path in `app.rs`, delegates in `client.rs`) — **not runtime-verified** (missing `mac` module + binary entry points + `build.rs`) |
| DevTools opens? | **Not yet tested** — `ShowDevTools` API not called anywhere; `on_popup_browser_view_created` has `_is_devtools` param present but unused |
| Passkey login works? | **Not yet tested** — requires signed app bundle with entitlements; current `project.yml` disables code signing; Chromium 149 has full WebAuthn L3 support |
| Adblock blocks on request path? | **Not yet prototyped** — no `adblock` dependency, no `ResourceRequestHandler` implementation; the `cef` crate's `request_handler` coverage needs verification |
| Focus/routing issues? | **Not yet testable** — no `NSViewRepresentable` exists; Views framework creates its own window (correct for spike but needs rework for SwiftUI hosting in M2); thread dispatch pattern is correct |
| Go/No-go for MVP1? | **GO** — clear 6 prerequisites (3 missing files + build.rs + resources.rs + compile verification) before M1; all are code-completion tasks, not architectural changes |
