# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

**Ochre** - a minimal, fast, cross-platform browser: **Arc's interaction model with native OS chrome.** the system web engine (WebKit on macOS, ADR-0072/0074), a shared Rust core, per-platform native UI (SwiftUI + Liquid Glass on macOS; WinUI 3 + Fluent on Windows, future). ~80% of the code lives in the Rust core; native layers are view + binding only. The shipped name is `product.toml` (ADR-0060); crates, FFI, Swift types, and the Xcode scheme stay `browser` / `Browser`. To rename the product, follow `.claude/skills/rename-product/SKILL.md`.

Current focus is **MVP1 (macOS)** - see `docs/mvp1.md` for the M1–M5 milestones. The full design is `docs/architecture.md` (§-numbered; other files reference these sections).

## Steering & roadmap

The project self-steers through a **strategy / execution split** plus a harness skill (ADR-0001):

- **`docs/roadmap.md`** - *strategy*: thesis, positioning, the phased plan (§5), the open forks (§10). Slow-changing; the *why* and *in what order*.
- **`docs/backlog.md`** - *execution*: a rolling, status-tracked queue of concrete scoped items. The *what's the next thing*. Keep it honest - mark items `done` as they land, file discovered work as `next` rows (never a silent TODO).
- **`docs/parity.md`** - *coverage*: the capability × frontend status grid (ADR-0007), each row classed Invariant/Idiom per the consistency contract (`style §2`). The source of truth for *where we are on cross-frontend parity*; kept in sync as frontend-facing capabilities land.
- **`steer` skill** (`.claude/skills/steer/SKILL.md`) - the harness. It reads both docs and routes: **"what's next?"** → propose + scope the next item, then stop for a go-ahead; **"add X / fix Y"** → file it in the backlog, then do it; **vision talk** → capture to the roadmap, then decompose into backlog items. When a frontend-facing capability lands, flipping its `parity.md` cell is part of marking it done. Invoke it (or follow its procedure) for any of those three.
- **`crash` skill** (`.claude/skills/crash/SKILL.md`) - **"why did it crash?"** / app quit / pasted `.ips` / SIGSEGV/panic. Dumps first (`just crash-latest`), match `docs/crashes/`, explain the mechanism, then fix. Mandatory for those triggers (see Gotchas).
- **`rename-product` skill** (`.claude/skills/rename-product/SKILL.md`) - change the user-facing name. Edit `product.toml`, `just sync-product`, `just product-check`. Do not rename crates / FFI / Swift types.

## Development principles

- **Parallel work is normal.** Multiple agents may be working in this repo at the same time. The tree can be temporarily broken - missing symbols, half-wired FFI, Swift/Rust mismatches - while another session's slice is still in flight. That is expected; do not treat it as your bug to fix. Stay scoped to your task and the files you own. Do not pick up another agent's unfinished work, stub missing pieces, or chase a green build outside your slice just to get `just verify` to pass. If a failure clearly comes from unrelated in-progress changes, note it and move on; the owning session will land the rest.
- **Use work leases around slices.** Multiple agents edit in parallel; do not compile or test while another session is still editing - the tree is likely broken. At slice start run `just work-begin "task label"`; when edits are done run `just work-end`, then `just verify` / `just build` / `just run`. Those recipes wait until every lease clears (Cargo already serializes simultaneous compiles - no extra compile mutex needed). Long slices: `just work-touch`. Check `just work-status` if a build seems stuck waiting. **Parallel agents:** set a unique `BROWSER_WORK_SESSION` per conversation (e.g. `export BROWSER_WORK_SESSION=agent-$(uuidgen | cut -c1-8)`); a lone agent can omit it (cookie in `target/.work-leases/.session-cookie`). Set `BROWSER_WORK_LEASE=0` to bypass (debug only).
- **Work autonomously.** The scope is well-defined - `docs/architecture.md`, `docs/mvp1.md`, and established browser conventions answer most "what should this do" questions. Implement, verify, and report; don't stop for direction on well-understood features. Reserve questions for genuine forks where multiple valid designs exist and the call is the user's to make.
- **Build features end-to-end.** A feature isn't a core function, or an FFI symbol, or a lone menu item - it's the whole vertical slice wired through and working: Rust core → FFI → native binding → menu/shortcut → visible, usable behavior. Default to shipping the complete path, and *steer the user toward it* - when a request would land as a partial slice (core-only, no UI; a binding with no backing logic; a stub "for now"), say so and drive it to the full end-to-end version rather than stopping halfway. **Verify autonomously as you go** with the tools on hand - `just verify`, `just run`, `just focus-selftest`, the float-the-window screenshot protocol - don't declare it done until you've observed the slice actually work end-to-end. If a slice genuinely can't be finished in one pass, flag the gap explicitly (backlog `next` row, never a silent TODO); don't quietly leave it dangling.
- **Stay lean.** Minimal and fast is the whole point. No speculative abstractions, no dead code, no dependency you don't need. Three similar lines beat a premature abstraction - cut before you add.
- **Core logic is Rust and frontend-agnostic.** All browser functionality - tabs, spaces, history, adblock, vault, automation - lives in `crates/core` behind a coarse FFI surface. Native layers are view + binding only. Frontend roadmap: SwiftUI/macOS now; later a vitae (Rust) frontend replaces it at proven parity and covers Windows/Linux (ADR-0064). Never leak platform-specific logic into the core, and never put logic in a frontend that a second frontend would have to reimplement.
- **Match established browser UX.** For user-facing behavior - keyboard shortcuts, tab handling, address-bar semantics, navigation - default to what mainstream browsers (Chrome, Safari, Arc) already do. Deviate only with a deliberate reason.
- **Production-grade UI, always.** Every shipped surface should feel finished and consistent - no placeholder states, no rough edges left "for now." If a feature can't be made production-quality yet, flag it instead of shipping a half-version. UI/UX rules live in `docs/style-guide.md` (native materials, Arc interaction model, the cross-platform consistency contract, keyboard/power-user conventions) - follow it and extend it when a new decision is made.
- **Every fix adds a mechanical guard** (ADR-0065). A bug fix or crash post-mortem is not done until it adds something a machine runs that would have caught it: a clippy/dylint lint, a source check in `just verify` (like `emdash-check`), a unit or property test, or a selftest. If no guard is possible, say why in the post-mortem or commit.
- **No em-dashes, anywhere.** The em-dash character (U+2014) is banned in every file in this repo: docs, ADRs, code comments, UI strings, log messages, release notes, commit messages. Use a period, comma, colon, parentheses, or a plain hyphen instead. Enforced by `just emdash-check` (part of `just verify`).

## Decision log (ADRs)

Durable decisions live in `docs/adr/` as numbered Architecture Decision Records - the persistent, git-committed rationale any agent or human can reference across sessions. CLAUDE.md and the spec say *what* the system is; ADRs say *why it's that way*.

- **Read before deciding.** Before making or reopening a non-trivial design choice, skim `docs/adr/README.md` (the index) so you don't re-litigate a settled question or re-explore a rejected path.
- **Write when a decision is made** - any time you choose among alternatives, defer or cut scope, or reverse an earlier approach (architectural or not). Do it at decision time, not "later"; the reasoning is lost to context compaction. One decision per ADR.
- **Format** (`docs/adr/NNNN-kebab-title.md`, copy `0000-template.md`): `Status` (Proposed / Accepted / Superseded by ADR-XXXX), `Context`, `Decision`, `Alternatives` (what was rejected and *why* - the part that saves future agents), `Consequences`.
- **Immutable.** Don't rewrite an accepted ADR's decision - supersede it with a new one and flip the old status. Update the index. Commit ADRs alongside the code that realizes them.

## Version control: git

This repo is **git**. It stays on git until Aether is ready (ADR-0065).

- Use normal git commands (`status`, `add`, `commit`, `switch`, `stash`, `rebase`, `push`).
- **`.gitignore` must cover new build outputs before you generate them.** `/target`, the generated `apps/macos/Browser.xcodeproj`, and `apps/macos/build.noindex/` (the `.noindex` suffix keeps build copies of the app out of Spotlight) are already ignored; keep it that way when adding new ones.
- **Parallel agents** share one working tree. Isolation is `git worktree add` if you need a separate checkout; work leases (`just work-begin` / `work-end`) serialize builds, not directories. Do not `git reset --hard`, `git checkout` other people's files, or `git stash` the whole tree to clean up concurrent work.

## Commands

All via the `justfile` (delegates to `cargo` / `xcodebuild`):

| Task | Command |
|---|---|
| Build workspace | `just build` (`cargo build --workspace`) |
| Test | `just test` |
| Test one crate / single test | `cargo test -p browser-core` / `cargo test -p browser-core <name>` |
| Lint (clippy, `-D warnings`) | `just lint` |
| Format check / write | `just fmt-check` / `just fmt` |
| Full CI-style check | `just verify` (build + test + lint + fmt-check + emdash-check + product-check) |
| Work lease status | `just work-status` (who's still editing) |
| Claim / release lease | `just work-begin "task"` / `just work-end` |
| **Run the app** | `just run` (SwiftUI `{name}.app` from `product.toml` - currently `Ochre.app`) |
| macOS SwiftUI app (build only) | `just gen-macos` then `just build-macos` |
| Standing regressions (selftests) | `just focus-selftest` / `just drag-selftest` / `just close-selftest` / `just little-selftest` / `just newtab-selftest` |
| Release build → install to /Applications | `just build-release` / `just dogfood` |
| Latest local crash payload | `just crash-latest` (path), `--print` (contents), `--list` (newest few) |

### When to add a `just` recipe

The `justfile` is not a catalog of every useful command. **Agents work well with shell** - prefer documenting workflows as plain commands and file paths (see `docs/crashes/README.md`, crash diagnosis in Gotchas below) unless a recipe earns its place.

Add a `just` recipe only when it is:

1. **Human-facing** - a stable entry point humans run by muscle memory (`just run`, `just verify`, `just dogfood`). Discoverability in `just --list` matters more than saving agents a few keystrokes.
2. **Not reliably reproducible in bare shell** - multi-step orchestration agents get wrong often: work-lease gating (`with-work-gate.sh`), XcodeGen + xcodebuild wiring, throwaway `$HOME` for selftests. The recipe encodes ordering and env the agent would otherwise have to re-derive from docs every time.

Do **not** add `just` wrappers for: reading docs, listing/searching git-committed files, scaffolding markdown from a template, or any one-liner an agent can run directly (`ls`, `cat`, `rg`, `cp`). Those belong in `CLAUDE.md` or the relevant `docs/` README as described shell.

`just crash-latest` stays because it picks debug vs release support-dir paths from `product.toml` - easy to get wrong by hand. Crash **post-mortems** (`docs/crashes/`) intentionally have no recipes; agents copy the template and `rg` the directory.

`just product-check` stays because the product name is one toml file plus many live surfaces; agents miss them. `just sync-product` applies the toml to the generated constants and `project.yml`.

`just run` builds and launches the native app for the current platform (macOS: the SwiftUI `{name}.app` from `product.toml`, WebKit driven by the Rust core via FFI). `scripts/prebuild-macos.sh` builds `libbrowser_ffi.dylib`; `scripts/bundle-macos-app.sh` embeds it in the `.app`. There is no engine to bundle: WebKit is part of macOS and is patched by macOS updates.

## Architecture

### Workspace layout
```
crates/core/   browser-core: WebKit engine wrapper (engine/) + all browser logic -
               tabs/spaces/session, history + favicons (rusqlite), adblock +
               cookie-consent, vault (Bitwarden-compatible), Arc import, private
               windows, tab suspension, settings/theme, crash reporting
crates/ffi/    browser-ffi:  C ABI bridge to native UIs (cdylib + rlib)
apps/macos/    the macOS app: AppKit window/chrome + SwiftUI islands (XcodeGen)
docs/          roadmap, backlog, parity, architecture, mvp1, style-guide, code-style,
               spike, adr/, crashes/ - see docs/README.md
```
Future workspace members (in the spec, not yet code): `crates/agent`, `apps/windows`.

### The engine (`crates/core/src/engine/`, ADR-0074)
The native app owns every window; for each tab it hands the core a container `NSView`, and the core inserts a `WKWebView` as its subview (`engine::create`). The tab model holds an `engine::Browser` - a plain id, `Send`, safe inside the global stores - and the WebKit objects live in a main-thread registry in `engine/mod.rs`. Every handle method is a no-op off the main thread or once the view is closed. One `WebDelegate` per view (`engine/delegate.rs`, objc2 `define_class!`) receives navigation, UI and KVO callbacks; `engine/ui.rs` supplies dialogs, the file picker, the error page and downloads; `engine/content_rules.rs` compiles the ad-block lists to WebKit content rules.

**Engine callbacks never run core code inline.** WebKit fires some (KVO on `URL`) synchronously inside our own calls, possibly while the tab-store mutex is held. Post every reaction through `crate::main_thread::post` / `post_local` (the libdispatch main queue), and copy a completion handler (`handler.copy()`) when the answer needs the tab model.

Profiles: normal tabs share WebKit's default persistent data store (per bundle id, so debug and release never share cookies); each private window gets a non-persistent store. Each profile has one `WKUserContentController` carrying the injection substrate (`inject.rs`: isolated `WKContentWorld` + reply-capable script message handler, all frames), the YouTube scriptlet, and the compiled rule list.

`root_cache_dir()` is `~/Library/Application Support/{name}` (from `product.toml`; debug builds use `{name} Dev`) for the core's own files (session, history, vault, settings). A previous product name listed in `legacy_names` is renamed into the current folder on first launch.

### FFI (`crates/ffi/src/lib.rs`)
A coarse-grained `extern "C"` / `#[no_mangle]` surface - ~120 functions in one file - over the core, built as `libbrowser_ffi.dylib` and called from Swift through `apps/macos/Sources/Browser-Bridging-Header.h` (declare every new export there too, or Swift won't see it). Beyond the embed lifecycle it covers tabs/spaces/folders, navigation, find, zoom, history/suggestions, private windows, vault, import, and settings/theme. State flows one way: Swift calls a mutator → the core emits a `tabs::Event` (`StructureChanged`, `TabTitleChanged`, …) through the callback registered with `browser_set_event_callback` → Swift re-pulls a JSON snapshot (e.g. `browser_structure_json()`). Returned strings are Rust-allocated `*mut c_char` - free them with `browser_string_free`.

### macOS app (`apps/macos`)
XcodeGen-based: **`project.yml` is the source of truth** (plaintext, agent-edited). `Browser.xcodeproj` is generated by `just gen-macos` and is gitignored/disposable - never hand-edit the `.pbxproj`. There is no SwiftUI `App` entry point: `Sources/main.swift` calls `browser_initialize()` **first** (crash reporting and core state load before any window exists), then installs `AppDelegate` and builds the window. AppKit owns the window and all top-level frames (`RootView.swift`); SwiftUI renders only inside hosted `NSHostingView` islands (sidebar, command bar, settings, history, vault). `BrowserState.swift` is the Swift-side observable mirror of core state, updated from FFI events; `Menu.swift` builds the menu bar.

## Gotchas & conventions

- **"Why did it crash?" → `crash` skill (mandatory).** Phrases like "why did it crash", "the app quit", a pasted `.ips`, SIGSEGV/panic, or "Browser died" are **not** open-ended chat. Invoke `.claude/skills/crash/SKILL.md` immediately: run `just crash-latest --list` / `--print`, read `diagnostics.log`, match `docs/crashes/CRASH-NNNN`, answer with mechanism + evidence paths, then **start fixing** (or present a real design fork). Do not ask the user to dig up dumps unless those paths are empty. Do not answer from a stale `latest.json` when a newer `.ips` exists (`--list` by mtime). Capture code: `crates/core/src/crash_report.rs`. Post-mortems when fixed: copy `docs/crashes/0000-template.md` → next `NNNN-….md`, index row, gotcha if standing. Canonical examples: **CRASH-0001**, **CRASH-0002**.
- **SwiftUI hit-testing - no full-size overlays on gesture trees** (CRASH-0001). A full-bleed `.overlay` on a `ScrollView` / tab list that already stacks `contentShape` + `onTapGesture` + `onDrag`/`onDrop` can recurse infinitely in `PlatformHitTestingManager` and blow the stack. Sidebar window-drag suppression: `RootView.installSidebarTabDragWindowGuard` (AppKit), not SwiftUI overlays. Prefer `.contentShape(.rect)` on dense lists; `.allowsHitTesting(false)` on hidden drag indicators.
- **Verifying rendering from an agent session - establish observability BEFORE diagnosing.** Web engines throttle occluded windows (Chromium stopped producing frames entirely). An app launched from a background shell lands behind the user's windows, so screenshots show a blank web area even though the page loaded fine (the load finished and the title binds). This is indistinguishable from a real rendering bug in a screenshot - an entire debugging session was once wasted chasing it. Tell-tale: gray traffic lights = window inactive = screenshot proves nothing. The reliable protocol (no focus stealing, no user interruption): lldb `[window setLevel:5]` → `screencapture -x -l<windowID>` → `setLevel:0`. Never conclude "the engine isn't rendering" - and never start "fixing" it - from a capture of a window that wasn't verifiably on top.
- **Never let SwiftUI position or restructure around the web view.** SwiftUI structural churn (`if`/transitions) and frame animations around an `NSViewRepresentable` hosting the engine view stall rendering or misplace the view outright. AppKit owns all top-level frames (`apps/macos/Sources/RootView.swift`); SwiftUI renders content only inside hosted `NSHostingView` islands.

- **Every user-facing feature gets a native menu item.** The platform menu bar is not optional chrome - on macOS it's how the OS routes ⌘-shortcuts and clipboard/undo (Cut/Copy/Paste/Select All dispatch through nil-targeted first-responder selectors into the web view), and it's the discoverability surface for every command. macOS commands live in `apps/macos/Sources/Menu.swift` (`NSApp.mainMenu`), wired to `AppDelegate` actions (or system selectors for standard Edit/Window/App items). So when you add a feature that has a keyboard path, add its menu item + shortcut there in the same change, mirror the shortcut into `docs/style-guide.md §9`, and gray it out via `AppDelegate.validateMenuItem` when it's unavailable - a shortcut with no menu item is a bug, not a shortcut. Future frontends (Windows/Linux) get the equivalent native menu; the actions stay in the core/FFI so the menu is view-and-binding only.
- **`docs/` is the design set** - see `docs/README.md` for the map and the cross-ref convention (`spec §N` = architecture.md, `rm §N` = roadmap.md, `mvp` = mvp1.md, `style §N` = style-guide.md, `code §N` = code-style.md, bare `§N` = the current doc). `docs/spike.md` is the historical CEF engine reference; the WebKit move is ADR-0072 / ADR-0074 and `docs/research/2026-11-webkit-spike.md`.
- **Not yet built** (spec'd but absent): passkeys on a **signed, entitled** bundle (P0-2: WebKit's WebAuthn for arbitrary sites needs Apple's `com.apple.developer.web-browser.public-key-credential`), extensions, Widevine (WebKit has FairPlay instead), `crates/agent`, the Windows frontend, vault passkeys (P2-50), page automation (CDP is gone; P1-52). Most of the rest of the spec **is** built - tabs/spaces, adblock, cookie consent, vault + autofill, history/favicons, Arc import, private windows, Web Inspector - so check `docs/backlog.md` and `docs/parity.md` for current status before assuming a capability is missing.
- Keyboard/focus routing between the web view and SwiftUI was the highest-risk MVP item (spec §15); it is now **proven** and guarded by a standing regression - `just focus-selftest` (in-process synthetic input, ADR-0018). Re-run it after any engine or focus-path change; don't regress it silently.
- Sidebar drag-and-drop is a **custom gesture-driven layer** (ADR-0035) - no `onDrag`/`onDrop`, one frame-based resolver, drag visuals in a non-hit-testable overlay. Guarded by a standing regression - `just drag-selftest`; re-run it after sidebar layout or drag-path changes. Any AppKit `NSViewRepresentable` spanning the tab list must be hit-test-inert (`hitTest → nil`) or it swallows every SwiftUI click/gesture behind it.
- **Tab closes must never close the window**: closing a tab's web view (`engine::Browser::close`) only detaches and releases that view; ⌘W on the empty canvas closes the window (Arc semantics). A page's own `window.close()` arrives as `webViewDidClose` and drops the tab through the posted `note_closed` path. Guarded by a standing regression - `just close-selftest`; re-run it after close-path, suspension, or engine-lifecycle changes. Related: never ask `NSApp.currentEvent` for key-event-only properties without checking `event.type` (CRASH-0002).
- **Restricted entitlements need a provisioning profile** (ADR-0062 / ADR-0070). Never ad-hoc-sign `Browser.production.entitlements` (`com.apple.developer.web-browser*`): AMFI SIGKILLs the binary at exec (CRASH-0004). Developer ID inside-out signing is `scripts/codesign-macos-app.sh` (called from `just build-release`), full hardened runtime with only the device entitlements; never `codesign --deep`. The remaining P0-2 gate is Apple granting the passkey capability (Account Holder form: `macos-browsers-passkeys`).
- **WebKit reports page URLs in canonical, percent-encoded form** (`data:text/html,%3Ch1%3E...`). Compare URLs in that form; selftest fixtures already use it.
- **"It feels off" reports → friction bundles.** Help > Report a Problem (`Cmd+Opt+B`, P1-44) writes `{support}/friction/<unix-ms>/` (support = `~/Library/Application Support/Ochre Dev` for debug, `Ochre` for release): `report.json` (note, focused surface, visible URL, version), `structure.json` (the tab snapshot), `diagnostics.log` (log tail), `window.png`. When Korbin mentions a reported problem, read the newest bundle first: `ls -t ~/Library/Application\ Support/Ochre*/friction | head`.
- Follow the existing module-doc style (`//!` headers explaining the *why*, especially for `unsafe`/objc2 code) and keep the FFI surface coarse-grained.
