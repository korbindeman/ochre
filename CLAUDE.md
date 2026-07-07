# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A minimal, fast, cross-platform browser: **Arc's interaction model with native OS chrome.** Chromium engine via CEF, a shared Rust core, per-platform native UI (SwiftUI + Liquid Glass on macOS; WinUI 3 + Fluent on Windows, future). ~80% of the code lives in the Rust core; native layers are view + binding only.

Current focus is **MVP1 (macOS)** — see `docs/mvp1.md` for the M1–M5 milestones. The full design is `docs/architecture.md` (§-numbered; other files reference these sections).

## Steering & roadmap

The project self-steers through a **strategy / execution split** plus a harness skill (ADR-0001):

- **`docs/roadmap.md`** — *strategy*: thesis, positioning, the phased plan (§5), the open forks (§10). Slow-changing; the *why* and *in what order*.
- **`docs/backlog.md`** — *execution*: a rolling, status-tracked queue of concrete scoped items. The *what's the next thing*. Keep it honest — mark items `done` as they land, file discovered work as `next` rows (never a silent TODO).
- **`docs/parity.md`** — *coverage*: the capability × frontend status grid (ADR-0007), each row classed Invariant/Idiom per the consistency contract (`style §2`). The source of truth for *where we are on cross-frontend parity*; kept in sync as frontend-facing capabilities land.
- **`steer` skill** (`.claude/skills/steer/SKILL.md`) — the harness. It reads both docs and routes: **"what's next?"** → propose + scope the next item, then stop for a go-ahead; **"add X / fix Y"** → file it in the backlog, then do it; **vision talk** → capture to the roadmap, then decompose into backlog items. When a frontend-facing capability lands, flipping its `parity.md` cell is part of marking it done. Invoke it (or follow its procedure) for any of those three.

## Development principles

- **Work autonomously.** The scope is well-defined — `docs/architecture.md`, `docs/mvp1.md`, and established browser conventions answer most "what should this do" questions. Implement, verify, and report; don't stop for direction on well-understood features. Reserve questions for genuine forks where multiple valid designs exist and the call is the user's to make.
- **Build features end-to-end.** A feature isn't a core function, or an FFI symbol, or a lone menu item — it's the whole vertical slice wired through and working: Rust core → FFI → native binding → menu/shortcut → visible, usable behavior. Default to shipping the complete path, and *steer the user toward it* — when a request would land as a partial slice (core-only, no UI; a binding with no backing logic; a stub "for now"), say so and drive it to the full end-to-end version rather than stopping halfway. **Verify autonomously as you go** with the tools on hand — `just verify`, `just run` / `just run-native-shell`, `just focus-selftest`, the CDP harness, the float-the-window screenshot protocol — don't declare it done until you've observed the slice actually work end-to-end. If a slice genuinely can't be finished in one pass, flag the gap explicitly (backlog `next` row, never a silent TODO); don't quietly leave it dangling.
- **Stay lean.** Minimal and fast is the whole point. No speculative abstractions, no dead code, no dependency you don't need. Three similar lines beat a premature abstraction — cut before you add.
- **Core logic is Rust and frontend-agnostic.** All browser functionality — tabs, spaces, history, adblock, vault, automation — lives in `crates/core` behind a coarse FFI surface. Native layers are view + binding only. Frontend roadmap: SwiftUI/macOS (MVP1) → Windows → Linux. Never leak platform-specific logic into the core, and never put logic in a frontend that a second frontend would have to reimplement.
- **Match established browser UX.** For user-facing behavior — keyboard shortcuts, tab handling, address-bar semantics, navigation — default to what mainstream browsers (Chrome, Safari, Arc) already do. Deviate only with a deliberate reason.
- **Production-grade UI, always.** Every shipped surface should feel finished and consistent — no placeholder states, no rough edges left "for now." If a feature can't be made production-quality yet, flag it instead of shipping a half-version. UI/UX rules live in `docs/style-guide.md` (native materials, Arc interaction model, the cross-platform consistency contract, keyboard/power-user conventions) — follow it and extend it when a new decision is made.

## Decision log (ADRs)

Durable decisions live in `docs/adr/` as numbered Architecture Decision Records — the persistent, git-committed rationale any agent or human can reference across sessions. CLAUDE.md and the spec say *what* the system is; ADRs say *why it's that way*.

- **Read before deciding.** Before making or reopening a non-trivial design choice, skim `docs/adr/README.md` (the index) so you don't re-litigate a settled question or re-explore a rejected path.
- **Write when a decision is made** — any time you choose among alternatives, defer or cut scope, or reverse an earlier approach (architectural or not). Do it at decision time, not "later"; the reasoning is lost to context compaction. One decision per ADR.
- **Format** (`docs/adr/NNNN-kebab-title.md`, copy `0000-template.md`): `Status` (Proposed / Accepted / Superseded by ADR-XXXX), `Context`, `Decision`, `Alternatives` (what was rejected and *why* — the part that saves future agents), `Consequences`.
- **Immutable.** Don't rewrite an accepted ADR's decision — supersede it with a new one and flip the old status. Update the index. Commit ADRs alongside the code that realizes them.

## Commands

All via the `justfile` (delegates to `cargo` / `xcodebuild`):

| Task | Command |
|---|---|
| Build workspace | `just build` (`cargo build --workspace`) |
| Test | `just test` |
| Test one crate | `just test-one core` → `cargo test -p browser-core` |
| Single test | `cargo test -p browser-core <name>` |
| Lint (clippy, `-D warnings`) | `just lint` |
| Format check / write | `just fmt-check` / `just fmt` |
| Full CI-style check | `just verify` (build + test + lint + fmt-check) |
| **Run the app** | `just run` (SwiftUI `Browser.app` — the product) |
| Engine-isolation run | `just run-native-shell` |
| Assemble the standalone bundle | `just bundle` |
| macOS SwiftUI app (build only) | `just gen-macos` then `just build-macos` |

`just run` builds and launches the native app for the current platform (macOS: the SwiftUI `Browser.app`, embedded CEF via FFI). `just run-native-shell` is a debug-only bisection tool: a pure-Rust AppKit `NSWindow` hosting the embedded browser via `crates/core/src/native_shell.rs` (the same `embed.rs` hosting path the app uses, minus SwiftUI + FFI) — use it to tell whether a blank page / CEF issue lives in the engine or the Swift layer. The standalone `browser` binary in `target/bundle/browser.app` also accepts `--url=https://example.com`, `--use-alloy-style` (ALLOY runtime style), `--use-native` (native popup), and `--native-shell`; run it directly for the CEF-Views path (no dedicated recipe).

## Critical: CEF must run from a `.app` bundle

On macOS CEF loads its framework and helper executables from `Browser.app/Contents/Frameworks` at runtime — it is **not** linked. So `cargo run --bin browser` will not work. `just run-native-shell` (and `just bundle`) first runs `cargo run --bin bundle`, which uses `cef::build_util::mac::build_bundle` to assemble `target/bundle/browser.app` (browser + browser_helper + framework + per-role helper apps), then launches the binary *inside* the bundle so stdout/stderr stay in your terminal. The SwiftUI `just run` path has its own equivalent: `scripts/prebuild-macos.sh` assembles the bundle and `scripts/bundle-macos-app.sh` injects the CEF framework + helpers into `Browser.app`.

Notes:
- There is **no `build.rs`**. The CEF prebuilt binary distribution is downloaded by `cef-dll-sys`'s own build script at compile time (needs `cmake` + `ninja`). The `bundle` binary only lays out the `.app`.
- `build_bundle` always bundles the **debug** binaries (`target/debug`) — it's a debug-only dev convenience.

## Architecture

### Workspace layout
```
crates/core/   browser-core: CEF engine wrapper + lifecycle (the brain)
crates/ffi/    browser-ffi:  C ABI bridge to native UIs
apps/macos/    SwiftUI app (XcodeGen; currently a placeholder shell)
docs/          roadmap, backlog, architecture, mvp1, style-guide, spike, adr/ — see docs/README.md
```
Future workspace members (in the spec, not yet code): `crates/agent`, `apps/windows`.

### Two CEF hosting modes — this is the key structural split
- **Standalone** (`crates/core/src/lib.rs::run`): CEF's Views framework creates and owns its own `NSWindow`. This is the `browser` binary's default path (run it directly from the bundle — no dedicated `just` recipe). The product doesn't use it; it's a CEF-Views spike leftover kept only for engine smoke-testing.
- **Embedded** (`crates/core/src/embed.rs`): the native app owns the `NSWindow` and hands CEF a child `NSView` to render into (an ALLOY / chromeless browser). This is the "Option B" hosting model the SwiftUI app will use — driven across the FFI boundary (`initialize` → `create_browser(parent_view)` → `run_message_loop` → `shutdown`).

### Process model
`browser` (main) and `browser_helper` (renderer/GPU/utility subprocess) are **both** thin binaries that call `browser_core::run()`. The role is dispatched by CEF's `--type` command-line switch: helper processes return after `execute_process`; the browser process continues to `initialize` + `run_message_loop`. The helper executable lives one bundle level deeper, so `mac::load_cef_framework` resolves a different relative framework path when it sees `--type=`.

### macOS platform layer (`crates/core/src/mac.rs`)
Two hard CEF requirements live here, and both must happen on the **main thread before CEF init**:
1. **Framework loading** via `LibraryLoader` (guard that unloads on drop — keep it alive for the process).
2. **A CEF-conformant `NSApplication`**: a `BrowserApplication` subclass (built with `objc2`'s `define_class!`) that conforms to `CefAppProtocol` and tracks `-sendEvent:` — the Rust equivalent of `CefScopedSendingEvent`. A stock `NSApplication` would crash on CEF's `isHandlingSendEvent` selector. The send-event flag is a module `AtomicBool`, not an ivar, because `+sharedApplication` bypasses objc2 ivar init.

`root_cache_dir()` pins the CEF cache to `~/Library/Application Support/Browser` — an **absolute** `root_cache_path` is required, otherwise repeated dev launches collide on Chromium's process singleton and the second launch just hands off and exits.

### CEF handler pattern (`crates/core/src/client.rs`, `app.rs`)
The `cef` crate's `wrap_client!` / `wrap_*_handler!` / `wrap_window_delegate!` / `wrap_task!` macros generate the CEF trait impls (and hidden `cef_object` field + `new()` constructor) — implement handlers in idiomatic Rust inside these macros, don't hand-write C FFI shims. Shared state is one `Arc<Mutex<BrowserHandler>>` cloned into every handler (display/lifespan/load). All browser/host operations must run on `ThreadId::UI`; off-thread calls re-post themselves via `post_task` (see `show_main_window` / `close_all_browsers`).

**Runtime style** (`RuntimeStyle::DEFAULT` vs `ALLOY`) threads through app/client/delegates. ALLOY enables custom HTML error pages and the platform title/show paths; DEFAULT (Views) lets CEF manage the window and title.

### FFI (`crates/ffi/src/lib.rs`)
A thin `extern "C"` / `#[no_mangle]` surface (`browser_initialize`, `browser_create_browser`, `browser_run_message_loop`, `browser_quit`, `browser_shutdown`) over `browser_core::embed`. Built as a `cdylib` the SwiftUI app links against. The spec (§5) plans richer UniFFI/swift-bridge event streams (`TabTitleChanged`, etc.) for the tab model — not yet built; the current surface is just the embed lifecycle.

### macOS app (`apps/macos`)
XcodeGen-based: **`project.yml` is the source of truth** (plaintext, agent-edited). `Browser.xcodeproj` is generated by `just gen-macos` and is gitignored/disposable — never hand-edit the `.pbxproj`. `Sources/BrowserApp.swift` is currently a placeholder slim-chrome shell; the CEF `NSView` embed via `NSViewRepresentable` lands in MVP milestone M2.

## Gotchas & conventions

- **Verifying rendering from an agent session — establish observability BEFORE diagnosing.** Chromium suspends frame production for occluded windows, and it stays suspended even when `NSWindow.occlusionState` reads "visible". An app launched from a background shell lands behind the user's windows, so screenshots show a blank web area even though the page loaded fine (`on_load_end status=200`, title binds). This is indistinguishable from a real rendering bug in a screenshot — an entire debugging session was once wasted chasing it. Tell-tale: gray traffic lights = window inactive = screenshot proves nothing. The reliable protocol (no focus stealing, no user interruption): lldb `[window setLevel:5]` → `screencapture -x -l<windowID>` → `setLevel:0`. Never conclude "CEF isn't rendering" — and never start "fixing" it — from a capture of a window that wasn't verifiably on top.
- **Never let SwiftUI position or restructure around the CEF platform view.** SwiftUI structural churn (`if`/transitions) and frame animations around the CEF `NSViewRepresentable` stall Chromium's frame pipeline or misplace the view outright. AppKit owns all top-level frames (`apps/macos/Sources/RootView.swift`); SwiftUI renders content only inside hosted `NSHostingView` islands.

- **Every user-facing feature gets a native menu item.** The platform menu bar is not optional chrome — on macOS it's how the OS routes ⌘-shortcuts and clipboard/undo (Cut/Copy/Paste/Select All dispatch through nil-targeted first-responder selectors into CEF's web view), and it's the discoverability surface for every command. macOS commands live in `apps/macos/Sources/Menu.swift` (`NSApp.mainMenu`), wired to `AppDelegate` actions (or system selectors for standard Edit/Window/App items). So when you add a feature that has a keyboard path, add its menu item + shortcut there in the same change, mirror the shortcut into `docs/style-guide.md §9`, and gray it out via `AppDelegate.validateMenuItem` when it's unavailable — a shortcut with no menu item is a bug, not a shortcut. Future frontends (Windows/Linux) get the equivalent native menu; the actions stay in the core/FFI so the menu is view-and-binding only.
- **`docs/` is the design set** — see `docs/README.md` for the map and the cross-ref convention (`spec §N` = architecture.md, `rm §N` = roadmap.md, `mvp` = mvp1.md, `style §N` = style-guide.md, bare `§N` = the current doc). `docs/spike.md` is the CEF engine reference (crate choice, hosting model, passkey/adblock notes); the build-blockers it once listed are all resolved and the workspace compiles.
- **Not yet implemented** (spec'd but absent): the tab/space model, adblock on the request path, vault, DevTools API surface, CDP automation, passkey entitlements. Code signing is disabled in `project.yml` (`CODE_SIGNING_ALLOWED: NO`) for headless builds — passkeys need a signed bundle to test.
- Keyboard/focus routing between the CEF `NSView` and SwiftUI was the highest-risk MVP item (spec §15); it is now **proven** and guarded by a standing regression — `just focus-selftest` (in-process synthetic input, ADR-0018). Re-run it after any CEF bump or focus-path change; don't regress it silently.
- **CEF is pinned to the 148 line (`148.4.0`), not the latest 149** — Chromium 149 regressed subresource redirect handling (the request after a 3xx dies with `net::ERR_INVALID_ARGUMENT`; it broke the signed-in Google avatar). ADR-0028. Don't bump to 149+ until upstream fixes it. This is guarded by a standing regression — `just redirect-selftest` (hermetic in-process redirect, `crates/core/src/redirect_selftest.rs`); **re-run it after any CEF bump** and only move the pin forward if it stays green.
- Follow the existing module-doc style (`//!` headers explaining the *why*, especially for `unsafe`/objc2 code) and keep the FFI surface coarse-grained.
