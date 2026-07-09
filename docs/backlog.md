# Backlog

**Role:** the *execution layer* beneath `roadmap.md`. The roadmap is **strategy** (what to build, why,
in what order); this is the **operational queue** that answers **"what's next?"** — concrete, scoped,
status-tracked items in a rolling near-term window. Agent-maintained: kept in sync as work lands.

**How this file is driven** — see the `steer` skill (`.claude/skills/steer/SKILL.md`) for the full
procedure. In short:

- **"what's next?"** → the agent picks the top `next` item, scopes it into a brief, and stops for your go.
- **"add X / fix Y"** → filed here as a `next` item, then done as normal work.
- **vision talk** → captured in `roadmap.md`, then decomposed into items here.

**Status vocabulary:**

- `next` — ready, no unmet dependency; the pool "what's next?" picks from.
- `wip` — in progress.
- `blocked` — waiting on another item or an external / user action (named in **Deps**).
- `done` — complete (date in Notes).
- `later` — belongs to a future phase; listed for dependency context, not yet pickable.

**Conventions:**

- **Rolling window.** Only the current + adjacent phase are tracked at item granularity. Later phases live
  in `roadmap.md §5`; pull them in here as they near — don't mirror the whole roadmap (keeps this lean).
- **IDs are stable.** `P<phase>-<n>`, or `P1-M<n>` for milestone-aligned items. Never renumber; mark `done`,
  don't delete — the queue is also the record of what was done.
- **One source of truth per fact.** Strategy and rationale stay in the roadmap / ADRs; this file holds
  status + scope pointers, not re-explained reasoning.
- **Refs:** `rm` = roadmap.md · `spec` = architecture.md · `mvp` = mvp1.md · `style` = style-guide.md.

---

## Phase 0 — Foundation (de-risk the thesis) · *closing*

Goal (rm §5.0): the thesis physically works on a signed build. **Exit:** one signed tab renders real sites,
opens DevTools, completes a passkey login, and blocks on the request path.

| ID | Item | Status | Est | Deps | Refs |
|----|------|--------|-----|------|------|
| P0-1 | CEF ↔ native keyboard / focus routing | done | ~60k | — | spec §15#1 · rm §5.0 · mvp M2 · ADR-0018 |
| P0-2 | Signed + hardened-runtime bundle → passkey / Touch-ID login | blocked | ~50k | Apple Developer enrollment + entitlement request (your action) | rm §5.0 / §6.1 |
| P0-3 | Native ad-block on the CEF request path (engine + request hook + per-site state + FFI) | done | ~35k | — | rm §5.0 · spec §11 · mvp M5 · ADR-0020 |
| P0-4 | CDP surface (loopback, consent-gated) | done | ~25k | — | rm §5.0 / §3.5 |

Notes (reconciled 2026-07-06 against the working tree):
- **P0-1 is the gating risk**, and is **partially built but not proven**. The three focus mechanisms are in
  place: (1) app shortcuts (⌘T/S/R/[/]/W) run through the **menu bar** as key equivalents
  (`MainMenu.build`/`AppDelegate`) — `performKeyEquivalent:` intercepts them before CEF; (2) the command bar
  is a key-taking borderless `.nonactivatingPanel` (`KeyablePanel`/`CommandBar.swift`) that becomes key
  rather than fighting the web view for first responder, handing focus back with `parentWindow.makeKey()` on
  close; (3) tab-switch drives CEF's own focus API (`host.set_focus(1)` in `tabs.rs::set_visible`). What's
  **unverified is the routing**: (a) focus returning to the page after the panel closes — note the asymmetry,
  tab-select calls CEF `set_focus` but command-bar-close only does `window.makeKey()` and *assumes* CEF
  re-takes first responder (the most likely failure); (b) click-to-focus into the web view; (c) sidebar↔page↔
  command-bar transitions; (d) `set_focus(1)` actually landing keys in a newly-selected tab.
- **P0-1 verification plan (agreed 2026-07-06).** Focus/keyboard resists the project's non-interactive harness
  (screenshots/occlusion tricks prove *rendering*, not live input routing). Staged: **(1)** add focus-transition
  instrumentation — a CEF `FocusHandler` (`on_got_focus`/`on_set_focus`/`on_take_focus`, `client.rs`) + host
  `set_focus` logging (`tabs.rs`) + Swift command-bar first-responder logging, all tagged `[focus]` — so every
  transition is an observable fact; **(2)** one interactive pass (user drives the checklist) for cheap ground
  truth; **(3)** *then* decide whether to build a synthetic-`CGEvent` driver as a standing focus-routing
  regression (valuable because the security treadmill re-breaks this surface on every CEF bump).
- **P0-1 stage-2 interactive pass ran (2026-07-06) — routing confirmed, one edge fixed.** The axes I most
  worried about are clean: page ↔ command-bar (`window.firstResponder=RenderWidgetHostViewCocoa` + paired
  `cef on_got_focus` on every close) and tab-switch (`host set_focus` → `on_set_focus source=system` →
  `on_got_focus` all line up, incl. rapid 1↔2 storms). **The ① asymmetry does *not* bite in the common case** —
  the command bar is a separate child window, so closing it and `makeKey()`-ing the parent restores the web
  view that was never un-focused. **But** the logs caught a non-deterministic edge: after a click into the
  SwiftUI **sidebar** left the bare `NSWindow` as first responder, command-bar close restored `NSWindow` (no
  `on_got_focus`) → keystrokes would fall on the floor until the next page click. **Fix landed:** command-bar
  close now drives CEF `set_focus` on the active tab explicitly (`embed::focus_active_tab` →
  `browser_focus_active_tab` FFI → called in `CommandBar.hide`), mirroring the tab-switch path and matching
  Safari/Arc (dismissing the address bar returns focus to the page). Builds green (core+ffi+macOS, clippy,
  fmt). Re-test automated as a standing regression — see below.
- **Landed 2026-07-07 (P0-1 → done) — synthetic-input focus-routing regression (ADR-0018).** The stage-3
  "decide whether to build a driver" fork is resolved: built one, as an **in-process** self-test
  (`--focus-selftest`, `apps/macos/Sources/FocusSelfTest.swift`), not an external `CGEvent` driver.
  Rationale (ADR-0018): posting OS events needs Accessibility trust + a frontmost app — neither grantable
  in the headless harness — so it drives input at the app's own dispatch entry points instead
  (`NSMenu.performKeyEquivalent` for ⌘T/⌘L, `NSApp.sendEvent` for clicks), which exercises the real
  `performKeyEquivalent:` / first-responder / hit-test path with no AX and no frontmost requirement (also
  immune to the occlusion trap — it tests input routing, not rendering). Asserts, after each transition,
  that the main window's `firstResponder` is the active tab's CEF view, corroborated by the core's `[focus]
  cef on_got_focus`. Five cases: tab-switch focus, click-to-focus, command-bar open/close refocus, the
  **bare-`NSWindow` regression** for the landed fix, and ⌘L inline-edit refocus. **Verified:** `just
  focus-selftest` — all cases PASS, exit 0; the trace shows the full `host set_focus → on_set_focus
  source=system → on_got_focus` chain per transition and `fr=NSWindow` (fail state) recovering to
  `fr=RenderWidgetHostViewCocoa` in the regression case, proving the gate can actually go red. This is now
  the standing check the CEF security-bump treadmill re-runs. **Fidelity boundary (ADR-0018):** the
  bare-window precondition is set via `makeFirstResponder(nil)` and dismissal via the controller's
  `hide()`/`endEditing()` (AppKit local key monitors aren't driven by `NSApp.sendEvent`); the refocus logic
  under test runs identically. Char-delivery *into* the page (vs. first-responder ownership) still wants a
  JS-eval readback — out of scope; first-responder + `on_got_focus` is the accepted proof. **Unblocks
  P1-M2** (and M3/M5 transitively).
- **P0-4:** DevTools is wired as a toggle (`browser_toggle_dev_tools`, ⌘⌥I + F12) plus a page context menu
  with **Inspect Element** (opens DevTools at the clicked node) — the "opens DevTools" Phase-0 exit criterion
  is met. The loopback, consent-gated **CDP automation** surface **landed 2026-07-07** (below).
  DevTools opens as a **separate window**, not docked inline — investigated end-to-end 2026-07-07 and found
  **infeasible on CEF 149** (Chrome-bootstrap DevTools must be Chrome-style; a child `NSView` forces Alloy and
  CHECK-crashes; a Chrome-style DevTools window ignores `CefWindowInfo` so it can't be created hidden for
  view-adoption → a stray visible window is unavoidable). Full rationale + rejected approaches in **ADR-0021**;
  revisit only if a future CEF adds embeddable/Alloy-style DevTools.
- **Landed 2026-07-07 (P0-4) — consent-gated CDP surface.** New `crates/core/src/cdp.rs`: a persisted
  consent state (`root_cache_dir()/cdp.json`, **off by default**) + a pure `switches()` that, only when
  enabled, adds `--remote-debugging-port=<port>` + `--remote-debugging-address=127.0.0.1` (**loopback-bound**
  — an open CDP port is a remote-control vector). Wired into `embed`'s `on_before_command_line_processing`
  (**browser process only**; helpers ignore the port), read at startup before `cef_initialize`. FFI:
  `browser_cdp_set_enabled` / `browser_cdp_is_enabled` (persisted; **takes effect next launch** — the port is
  a Chromium startup switch, not runtime-togglable). **Verified:** `just verify` green — 4 unit tests (switch
  construction enabled/disabled + loopback, config round-trip, default-disabled); 112 core tests; clippy
  `-D warnings` + fmt clean. **Pending running-browser check (screenshot-free, so tractable):** launch with
  `cdp.json` `{"enabled":true}` under an isolated `$HOME` (per [[verify-app-isolated-home]]) and confirm
  `curl http://127.0.0.1:9222/json/version` returns the CDP target list. **Note for P2-14:** the autofill
  injection substrate will issue CDP methods **internally** via CEF `ExecuteDevToolsMethod` (no external port
  needed); this external port is the automation/agent surface (rm §3.5), a separate concern.
- **Also already built** (single-browser, uncommitted): nav + back/fwd/reload/zoom, find-in-page, history
  frecency autocomplete + favicons, and the glass-sidebar shell scaffolding.
- **Adblock scope decided 2026-07-07 (user) — network + full control UI now, cosmetic as a fast-follow (ADR-0020).**
  "End to end + proper UI" decomposes into P0-3 (engine + request-path hook + per-site state + FFI), **P1-18**
  (control UI: per-site shield with live blocked count + allowlist popover beside the sidebar URL chip, plus
  global/per-site menu toggles), and **P2-21** (cosmetic element-hiding, deferred because doing it right needs
  the document-start injection substrate P2-14 — shared with autofill/Tweaks; injecting hide-CSS at load-end
  would flash, i.e. a half-version, so it waits). Decisions: **filter lists** = compiled-in seed for instant
  offline first-run + background EasyList/EasyPrivacy fetch that **hot-swaps** the engine mid-session (no weak
  first launch); **global RequestContext**, filter per-URL (no per-tab context refactor); per-site allowlist via
  the `psl` crate; state module mirrors `cdp.rs`. **Thread note:** `on_before_resource_load` runs on the CEF IO
  thread → engine behind `RwLock<Arc<Engine>>`, blocked count via `AtomicU64`. No filter-list settings page yet
  (minimal per the user); a full one stays Phase-2 settings-UI territory.

## Phase 1 — MVP1 (= mvp1.md) · *current focus*

Goal (rm §5.1 / mvp): the Arc interaction model in native chrome, good enough for **you** to daily-drive.
**Exit:** the mvp "Definition of done".

| ID | Item | Status | Est | Deps | Refs |
|----|------|--------|-----|------|------|
| P1-M1 | Core tab model + multi-tab CEF (Rust core + FFI event stream) | done | ~90k | — | mvp M1 · ADR-0003 |
| P1-6 | Empty state (zero-tab) UI + launch-to-empty | done | ~25k | P1-M1 | mvp (§new-tab); user request 2026-07-06 |
| P1-M2 | SwiftUI shell + vertical-tab glass sidebar | done | ~110k | P1-M1 · P0-1 (both done) | mvp M2 · ADR-0019 · closed 2026-07-08 |
| P1-M3 | Cmd+T command bar (Liquid Glass morph) | done | ~80k | P1-M2 | mvp M3 · closed 2026-07-08 |
| P1-M4 | Spaces switcher + multi-space UX | done | ~60k | P1-M1 | mvp M4 · ADR-0005/0006 |
| P1-7 | Tab folders + pinned/ephemeral split + folder↔space transform | done | ~110k | P1-M4 | ADR-0006; user request 2026-07-06 |
| P1-M5 | Adblock integration + polish (shortcuts, tab suspension, animation) | done | ~70k | P1-M2 (P0-3 covers the request-path core) | mvp M5 |
| P1-8 | Split `client.rs` (727 L) → `client/{handlers,favicon,devtools}.rs` — one file mixes 8+ CEF handlers, favicon capture, DevTools, error-page HTML | done | ~20k | — | code §3 |
| P1-9 | Split Swift view monoliths — `BrowserRootView.swift` (8+ view types) + `CommandBar.swift` (panel + key-handling + sizing) into per-component files | done | ~25k | — | code §3 |
| P1-10 | Omnibox-grade command-bar autocomplete — core fuzzy ranking + match highlighting + inline autocomplete | done | ~55k | — | ADR-0011 · style §6 · spec §9; user request 2026-07-06 |
| P1-11 | Sidebar layout: Arc control placement — nav controls share the traffic-light row, New Tab row heads the today section | done | ~15k | P1-6 | style §3; user request 2026-07-06 |
| P1-12 | Space emoji icons (core model + switcher UI) + default 🏠 Home space visible at launch (initial snapshot pull) | done | ~30k | P1-M4 | style §5; user request 2026-07-06 |
| P1-13 | History page polish — macOS 26 toolbar idiom (large title + Liquid Glass search capsule + glass Clear, `safeAreaBar` scroll-under), spacious rows + hover highlight | done | ~15k | — | style §15; user request 2026-07-06 |
| P1-14 | Traffic lights rework — custom-drawn in the sidebar, native buttons hidden; fixes drift/stale visibility from AppKit button relayout (worst on the beta OS) after three real-button approaches failed | done | ~90k | P1-11 | ADR-0012 · style §3; user bug report 2026-07-06 |
| P1-15 | Session persistence — tabs/spaces/folders survive restart (**all** tabs stay unless closed), lazy per-tab hydration | done | ~90k | P1-M4 | ADR-0013; user request 2026-07-06 |
| P1-16 | Persist sidebar pinned state across launches (UserDefaults, beside sidebar width) | done | ~10k | — | style §3; user request 2026-07-06 |
| P1-17 | Reopen closed tab (⌘⇧T) — faithful restore into original space/folder/pinned/slot, capped in-session stack | done | ~25k | P1-15 | style §4; user request 2026-07-06 |
| P1-18 | Adblock control UI — per-site shield (live blocked count + allowlist popover) beside the sidebar URL chip + global/per-site menu toggles | done | ~35k | P0-3 | ADR-0020 · style §16; user request 2026-07-07 |
| P1-19 | Auto-reveal collapsed sidebar when the empty canvas is showing — the empty (zero-tab) state floats the collapsed sidebar out so a saved/pinned tab can be picked, and keeps it out (hover-dismissal suspended) until a tab exists; scoped to the *visible* empty canvas so a full-content panel (History/Vault) over it tucks the sidebar away and closing the panel floats it back; covers launch-into-empty + close-last-tab | done | ~12k | P1-6, P1-16 | style §3; user request 2026-07-07 |
| P1-20 | App-UI surface ↔ tab mutual exclusivity — opening History/Passwords suppresses the sidebar active-tab highlight; clicking a tab dismisses the surface + focuses that tab; the two surfaces are exclusive with each other; leading ✕ close button on each; ⌘W dismisses whichever surface is open | done | ~15k | P1-13, P2-13 | ADR-0022 · style §15; user request/bug 2026-07-07 |
| P1-21 | URL-bar chip neutralized while an app-UI surface is open — the chip still shows the active tab's host while History/Passwords covers the canvas (the highlight is cleared but the address chip isn't); decide whether to blank/neutralize it for consistency (self-contained; core active tab stays as-is) | done | ~8k | P1-20 | ADR-0022 (discovered in P1-20) |
| P1-22 | Uniform button feedback — shared `IconButtonStyle` (hover fill + press-deepen, Reduce-Motion-aware) replaces the hand-rolled per-button highlights across the chrome; brings the URL-chip copy/shield buttons in line with the rest, adds the pressed state no custom button had, and generalizes action-confirmation (vault copy ✓ like the URL chip) | done | ~18k | P1-M2 | ADR-0023 · style §17; user request 2026-07-07 |
| P1-23 | Tab multiselect + folder-from-selection + clear ephemeral tabs (Arc) — ⌘/⇧-click range select, "New Folder from Selection", sidebar Today **Clear** (keeps multi-selection, else active tab); core `create_folder_with_tabs` + `clear_ephemeral_tabs` | done | ~35k | P1-7 | style §4; user request 2026-07-08 |
| P1-24 | Runtime tab suspension — auto-discard background CEF browsers (30 s grace on tab switch, immediate on space switch), favorites stay resident, `TabSuspended` event + sidebar dimming | done | ~45k | P1-15 | ADR-0033 · mvp risk #3; user request 2026-07-08 |
| P1-25 | Tab suspension settings + manual controls — grace duration, disable auto-suspend, "Suspend Other Tabs" menu | next | ~25k | P1-24 | ADR-0033 consequences |
| P1-26 | Session persist coalescing — debounce title/url write-through to `session.json` if profiling shows main-thread pressure | next | ~15k | P1-15 | ADR-0013 consequences |
| P1-27 | **Tab drag-and-drop reliability** — rebuilt the gesture layer: custom `DragGesture` drag (no `NSItemProvider`/`onDrop`), one frame-based resolver over the whole sidebar (no dead zones), overlay indicator + ghost (no mid-drag layout churn), synchronous commit on release, Esc cancel, edge autoscroll; standing regression `just drag-selftest` | done | ~40k | P1-7 · P1-23 | ADR-0035 (supersedes ADR-0015 gesture layer) · `TabDrag.swift` · `DragSelfTest.swift`; user bug report 2026-07-08, landed 2026-07-09 |
| P1-28 | **In-page address bar polish** — Safari-style viewport URL bar (`URLBar(placement: .viewport)`): fix interaction/focus bugs (⌘L routing, click-to-dismiss vs. pill hit-test, autocomplete popover, vault/shield flanking controls), layout/chrome-inset sync with CEF scroll-under; **page theme-color chrome always on** when viewport mode is enabled (not a separate toggle) — at `scrollY ≈ 0` fill the transparent strip behind the pill with the sampled color (`theme_color.rs` → `TabThemeColorChanged`); on scroll drop the matched fill so page pixels scroll up behind the bar (`CEFContainerView` scroll-under). Wire `BrowserState.themeColor` into the viewport chrome; interactive pass + extend `focus-selftest` for viewport mode | next | ~30k | P1-M2 · P0-1 | `URLBar.swift` · `RootView.swift` · `theme_color.rs`; **priority: polish soon** · user bug report 2026-07-08 |
| P1-29 | **`focus-selftest` red at HEAD** — cases `sidebar-empty-click-dismisses-urlbar` / `sidebar-empty-click-refocuses-page` fail on commit 326783a *before* any working-tree changes (first responder stuck on `_SystemTextFieldFieldEditor`; an empty-sidebar click no longer resigns the inline URL editor). Bisect against the P0-1 gate, fix, re-green | done | ~15k | P0-1 | ADR-0036 · `FocusSelfTest.swift`; found 2026-07-09 during P1-27, landed 2026-07-09 |
| P1-30 | **⌘W closed the window from an active saved/pinned tab** — CEF `DoClose` returning `false` `performClose:`s the browser's parent `NSWindow` (the app window) whenever a tab's view is still attached when the close pipeline lands; the hide/suspend paths raced their async view detach against it. `do_close` now returns `true` for host-embedded tabs; fixing it exposed and fixed a per-client `BrowserHandler` split (shared state — `browser_list`, `is_closing`, `close_all_browsers` — was per-tab) and CRASH-0002 (`isARepeat` on non-key events). Plus Arc ⌘W-on-empty-canvas: closes the window even with saved tabs in the sidebar. Standing regression: `just close-selftest` | done | ~60k | P1-M1 | ADR-0037 · CRASH-0002 · `client/mod.rs` · `AppDelegate.swift` · `CloseSelfTest.swift`; user bug report 2026-07-09, landed 2026-07-09 |
| P1-31 | **Pinned favorites are permanent (Arc), not dismissable** — ADR-0032 modeled a favorite "close" as *dismiss* (hide the icon into a three-dot "closed pinned tabs" menu + a −-on-hover button); this made pins vanish on ⌘W. Reversed: the `dismissed` concept is deleted end-to-end (core field/`SavedTab`/`TabView`/FFI `browser_tab_restore_favorite`/Swift `TabItem`+ellipsis+minus), `close_tab` on a favorite now just *deselects* (drops to the canvas, icon + loaded tab stay). Only Unpin/Delete remove one. Core test `favorite_close_deselects_but_keeps_icon` | done | ~25k | P2-36 | ADR-0038 (supersedes ADR-0032 dismiss semantics) · `tabs.rs` · `PinnedTabStrip.swift`; user bug report 2026-07-09, landed 2026-07-09 |

Notes (reconciled 2026-07-08):
- **Landed 2026-07-09 (P1-29) — focus-selftest re-greened; root cause was harness hermeticity, not a
  code regression (ADR-0036).** The throwaway `$HOME` in the selftest recipes does not isolate
  NSUserDefaults (cfprefsd resolves the logged-in user's domain regardless of the process
  environment), so the run inherited the developer's real `sidebarPinned.dev = 0` — the pinned
  sidebar host sat off-screen at `x = -chromeWidth` and the case-F synthetic click landed outside
  the window, hitting nothing. No commit broke it (a bisect would have found every commit red once
  the developer unpinned the sidebar while dogfooding); the product dismiss path was never broken.
  Fix: `FocusSelfTest` now opens with a `normalize-chrome-state` step asserting the layout its cases
  assume (`isSidebarPinned = true`, `isViewportURLBarEnabled = false`) via plain in-memory state
  assignments — never persisted, so real prefs stay untouched. **Verified:** `just focus-selftest`
  all 11 cases PASS; `defaults read com.nativechrome.Browser.dev` unchanged after the run.
- **Landed 2026-07-09 (P1-27) — sidebar drag-and-drop rebuilt on a custom gesture layer (ADR-0035).**
  The ADR-0015 *model* (implicit order, one `move_tab`, commit-on-drop) stands; the `onDrag`/`onDrop`
  gesture layer is gone. Root causes fixed, not patched: the layout-inserted placeholder created a
  resolve→shift→resolve feedback loop (flickering indicator, off-by-one drops); the per-row drop-
  delegate mosaic had dead zones — including the indicator itself — where a release silently vanished;
  and drag-end-by-polling raced `performDrop` (intermittent no-op commits). Now: rows feed a
  `DragGesture` (5 pt threshold) into `TabDragModel`; one resolver maps pointer → slot over frames
  reported in a single sidebar-wide coordinate space (strip resolved horizontally, list vertically,
  nearest-row fallback — every point yields a slot); all visuals are a non-hit-testable overlay
  (indicator line + pointer-following ghost); `onEnded` commits synchronously; Esc cancels (local key
  monitor); edge autoscroll via the enclosing `NSScrollView`. Library research (2025/26) confirmed no
  macOS package covers cross-section/folder/strip semantics — custom gesture drag is the production
  consensus (it's what Arc does); Apple's `reorderContainer(for:in:)` fixes this natively but only on
  macOS 27. **Verified:** new standing regression `just drag-selftest` — 9 cases green (reorder, self-
  drop no-op, save via New Tab boundary, file into folder, pin to empty strip, strip append, strip
  reorder, Esc cancel, unpin to open) driving the real resolver + core commit against live reported
  frames; `just focus-selftest` unchanged from its HEAD baseline (its two `sidebar-empty-click` cases
  were already red at 326783a — filed as **P1-29**, not from this slice). The one undriven layer is
  AppKit's `DragGesture` event delivery itself — worth one manual drag pass while dogfooding.
- **Landed 2026-07-08 (P1-M5 → done) — mvp M5 polish: shortcuts + animation tuning.**
  **Shortcuts (style §9):** core sidebar-order primitives in `tabs.rs`
  (`sidebar_tab_order`, `select_tab_by_index`, `select_adjacent_tab`,
  `select_adjacent_space`, `move_visible_tab`) + private-window equivalents;
  FFI + `Window` menu items with `validateMenuItem` gray-out. Wired: ⌘1…⌘8 / ⌘9
  (last), ⌘⇧[ / ⌘⇧], ⌘⌥← / ⌘⌥→ (spaces), ⌘⌥⇧↑ / ⌘⌥⇧↓ (move tab). Dismisses
  app-UI surfaces before tab/space select. **Animation:** sidebar pin/unpin
  duration aligned to the list spring (0.24s). **Verified:** 193 core tests (2
  new keyboard-order tests) + `just build-macos` **BUILD SUCCEEDED** +
  `just focus-selftest` all-PASS. **Parity:** shortcuts + sidebar collapse → ✅.
  **Deferred:** P1-25 (suspension settings UI); command-bar morph *feel* tuning
  (subjective `just run` pass); MVP DoD passkeys (P0-2 blocked on enrollment).
- **Landed 2026-07-08 (P1-M3 → done) — Cmd+T Liquid Glass morph (mvp M3).** Replaced the spring
  `scaleEffect` entrance with the platform's `glassEffectID` morph inside a `GlassEffectContainer`:
  present expands from a compact capsule to the full interactive field (`.regular.interactive()`),
  suggestions growing the same glass surface morph smoothly. `CommandBarPresentation` (observable)
  coordinates morph-out before panel teardown — `hide()` shrinks the glass, then fades/orders away.
  Reduce Motion honored (SwiftUI env + `MotionPolicy` for panel timing). **Verified:** `just
  build-macos` green; `just focus-selftest` — all command-bar cases PASS (open/close refocus +
  bare-window regression). **Pending your eye:** morph stiffness/feel wants a `just run` pass (can't
  be judged headless); parity expand-morph flipped to 🚧 until confirmed.

Notes:
- **Landed 2026-07-08 (P1-23 + P2-3) — tab multiselect, folder-from-selection, clear ephemeral.**
  **Core:** `tabs::create_folder_with_tabs` (batch pin+file, store-order preserved) and
  `tabs::clear_ephemeral_tabs` (bulk close, not reopenable; empty `keep` ⇒ active tab survives).
  FFI: `browser_create_folder_with_tabs`, `browser_clear_ephemeral_tabs`. **Swift:** ⌘-click toggles
  selection (seeds from the active tab on first ⌘-click), ⇧-click range-selects across the full sidebar
  order (folders → loose pinned → today), tint stroke for multi-selected rows; **Today → Clear** in the
  sidebar + File → Clear Ephemeral Tabs; File → New Folder from Selection + context-menu bulk actions
  (folder / close N tabs) when 2+ are selected. **Verified:** 4 new core unit tests + `cargo build`
  green; Swift compiles. **Pending interactive:** the ⌘/⇧ click gestures and Clear button want a `just run`
  pass (can't be driven headlessly).
- **Split view filed as P3-13** (`later`) — depends on stable multi-tab hosting + focus routing; not in
  scope for this slice.
- **P1-M1 is Rust-core only** — independent of focus routing, so it can run in parallel with P0-1.
- The event-stream boundary + **swift-bridge vs. UniFFI callback vs. extend the hand-rolled C ABI**
  decision lands inside P1-M1 (mvp M1 "Decision to lock") — write an ADR when it's chosen. Grounding
  (2026-07-06): no bridge dep is actually wired today (the "UniFFI" in `crates/ffi/Cargo.toml`'s
  description is aspirational text only); the working push channel is a hand-rolled C function-pointer
  callback (`browser_set_find_result_callback`) plus a title-via-NSWindow-KVO hack.
- **User request 2026-07-06 — "empty state → multi-tab → spaces, in that order."** Structurally these
  collapse onto the existing milestones: *empty state* (P1-6) and *multiple tabs* are the **same**
  structural change (a tab model where count is 0..N), delivered by **P1-M1** (the spine) + the tab-list
  slice of **P1-M2**; *spaces* is **P1-M4** (the M1 model is space-aware from day one, so M4 is the
  switcher UX + persistence). Delivery order this run: M1 → P1-6 + M2 tab-list → M4.
- **Landed 2026-07-06 (P1-M1 + P1-6).** Core `tabs` module (tab/space store, event dispatch); FFI
  event stream + `browser_tab_create/select/close` (hand-rolled C ABI, ADR-0003); **synchronous**
  `create_browser_sync` so `tab_id` = CEF browser identifier (no async correlation); per-tab
  visibility via `was_hidden`/`set_focus`; zero-tab-survival lifecycle (closing the last tab no longer
  quits — only window close / ⌘Q does). Swift: `BrowserState` tab model + event trampoline; `RootView`
  hosts one CEF view per tab + the native empty state (AppKit-owned); sidebar renders the live tab list
  (select / close / favicon / spinner); ⌘W closes the active tab. **Verified:** `just verify` green;
  macOS app builds; launch-to-empty confirmed (0 browsers at launch, no crash); tab creation +
  page-load confirmed via logs (`on_after_created`, `on_load_end status=200`). Reorder (drag) is M2;
  space create/switch/delete *commands* are M4. **Delivered against the tab-list slice; the glass
  sidebar chrome + focus-routing proof remain in P1-M2.**
- **Scope note — the delivered tab-list/empty-state UI is functional, not final glass chrome.** The
  full **P1-M2** (`NSGlassEffectView` sidebar, spring create/close/reorder animations, focus-routing
  proof P0-1) is still open.
- **Landed 2026-07-07 (P1-M2 wip) — tab animation polish + glass-material decision (ADR-0019).** Two
  parts. **(1) Glass, resolved not rebuilt:** the M2 "glass sidebar" line implied an `NSGlassEffectView`
  backing — that's wrong for macOS 26 and would violate `style §55`. The sidebar's material is the
  standard sidebar vibrancy, which macOS 26 auto-renders as Liquid Glass (sampling behind the *window*);
  discrete `.glassEffect` stays on the floating chrome (command bar / find / history) that composites
  over content correctly. The mvp's "glass lensing over content" is ruled out — in-window glass can't
  sample CEF's GPU layer (the `FloatingSidebar` already knew this) — so the *beside-content* fallback
  stands. Recorded as **ADR-0019**; `style §54` updated (fork closed). **No sidebar-material code change
  needed.** **(2) Animations, built:** the tab list gained real spring **enter/exit transitions**
  (`SidebarSelection.rowTransition` — grow-from-leading + fade in, fade out) and a **sliding active-tab
  highlight** (`matchedGeometryEffect`, shared `@Namespace` across loose/today/folder rows) so selecting
  a tab glides the pill between rows instead of cross-fading. Both ride the existing structural spring in
  `BrowserState.reloadStructure`, now gated on **Reduce Motion** (`style §7`). All contained in the
  SwiftUI islands — the CEF view swap stays AppKit-driven (CLAUDE.md rule). **Verified:** `just
  build-macos` green; `just focus-selftest` still all-PASS (tab create/select exercise the new
  animations without breaking lifecycle or focus). **Pending your eye (can't be judged headless — the
  point of the ⌘ note):** the *feel* — spring stiffness, transition distance, whether the selection pill
  flies distractingly on space-switch — wants a `just run` pass to tune. **Still open in M2:** that visual
  tuning pass; the **M3** command-bar `glassEffectID` morph (cross-window, hand-tuning-heavy) is untouched.
- **Landed 2026-07-06 (P1-M4 + P1-7) — spaces, folders, pinned/ephemeral, transforms.** Core `tabs`
  reworked into a tree (spaces → folders → tabs) with per-tab `pinned`/`folder`; new tabs are ephemeral
  ("today"), pinning/folders persist (ADR-0006). Event stream evolved (ADR-0005): hot content stays
  fine-grained, all structure changes emit one `StructureChanged` → the host re-pulls
  `browser_structure_json`. New FFI: pin, move-to-folder, folder + space CRUD, and the **bidirectional
  transform** (`folder_to_space` / `space_to_folder`, the latter closing the source space's ephemeral
  tabs). Swift: `BrowserState` rebuilt from the JSON snapshot; new `Spaces.swift` sidebar (bottom space
  switcher, collapsible folders, pinned/today sections, context menus for pin/move/transform/rename/
  delete; File → New Space ⌘⇧N / New Folder). **Verified:** `just verify` green; macOS app builds;
  runtime-exercised non-interactively (isolated `$HOME` + env hook) — tab-create → pin → create-folder →
  move-to-folder → new-space all round-tripped through the snapshot with no panic; JSON contract
  (`folder:null`, `activeTab:nil`) decodes cleanly. **Not yet runtime-driven by hand:** the transforms
  and deletes (same proven structural/snapshot machinery), and the sidebar's *visual* polish — those want
  an interactive pass (and land properly with P1-M2's glass chrome + P0-1 focus routing).
- **Deferred (filed, not silent):** ~~drag reorder / drag-between-folders~~ (landed 2026-07-06, below);
  drag-to-space-switcher; per-space color/theme (still out per mvp). (Cross-session persistence,
  deferred here, landed as P1-15.)
- **Landed 2026-07-06 — sidebar tab drag-and-drop (ADR-0015).** Reorder tabs inline within any
  section, pin/unpin by dragging across the today↔pinned boundary, and file a tab into/out of a
  folder (drop on the folder header). Core gained one primitive — `tabs::move_tab` /
  `TabStore::relocate_tab` — that re-parents (pin/folder, enforcing folder ⇒ pinned) and repositions
  in the store's `Vec` order in one structural change; order stays **implicit in store order** (no
  `order` field, no session migration). FFI: `browser_tab_move(id, pinned, folder, before)` with `-1`
  sentinels. Swift (`TabDrag.swift`): a local drop-*slot* model with a live insertion indicator,
  **committing one `move_tab` on drop** (never mutating the core mid-drag — it write-throughs on every
  change); the New Tab row doubles as the pin/unpin boundary so pinning works with an empty pinned
  section; reorder/create/close/select now spring-animate via `BrowserState.reloadStructure`.
  **Scope:** the three requested behaviors + folders; **drag-to-space-switcher stays deferred** (cross-
  space drag would need per-space active-pointer repair — noted in ADR-0015). **Verified:** `just
  verify`-equivalent green — 36 core tests (4 new: reorder / pin-into-folder / unpin / foreign-folder
  reject), clippy clean, macOS app builds. **Pending interactive:** the drag *gesture* itself (an
  NSItemProvider drag can't be driven headlessly) rides the standing P0-1 / M2 interactive pass.
- **User bug report 2026-07-08 — tab drag-and-drop is super buggy in practice.** The ADR-0015 slice
  landed the core primitive and a first Swift gesture pass, but day-to-day reorder / pin-unpin /
  folder filing is unreliable enough to block daily-driving. Filed as **P1-27** (priority: fix soon);
- **In-page address bar needs polish (user 2026-07-08; deferred 2026-07-08).** The Safari-style
  viewport URL bar (`Settings → Show in page`) is landed but **pill + flanking vault/shield controls
  are not reliably clickable** — sidebar-chip editing still works. Likely root cause: CEF draws behind
  the transparent strip (`clipsToBounds` off) and `PassthroughHostingView` / `ViewportURLBarHostingView`
  hit-test still passes clicks to the page when SwiftUI resolves to the hosting view (no AppKit child).
  Attempted: `onTapGesture` + `contentShape`, shared `ViewportGlassCapsule`, cluster-rect hit claiming
  in `ViewportURLBarHostingView` — still broken; needs a deeper pass (PreferenceKey-reported frames,
  AppKit overlay buttons, or CEF hit-test exclusion in the chrome band). Also: ⌘L routing,
  dismiss-on-viewport-click, autocomplete popover. Filed as **P1-28**; not blocking sidebar chip.
- **Page-aware chrome tint (user 2026-07-08).** Viewport address bar: **always** matches page
  theme-color when viewport mode is on — scroll-gated (matched fill at top, content scrolls behind
  on scroll). Sidebar: same sampled color, but **optional** (Settings toggle, off by default) and
  **no scroll-state** — static per-tab tint. Core sampling already streams via `theme_color.rs` /
  `TabThemeColorChanged`; Swift doesn't consume it yet. Viewport wiring is **P1-28**; sidebar is
  **P2-37**.
- **Landed 2026-07-06 (P1-15) — session persistence + lazy restore (ADR-0013).** History already
  persisted (SQLite); this adds the structural session: core `session.rs` write-through JSON
  (`session.json`, atomic tmp+rename on every structure/URL/title change — crash-safe, no save-on-quit
  step) + `TabStore::from_saved` sanitizing restore. `TabId` decoupled from CEF's identifier (core
  counter, persisted); `Tab.browser` is now `Option` — restored tabs are *suspended* and hydrate
  lazily via new `browser_tab_restore` FFI when first shown (`RootView.hydrateRestoredTab`), so N
  saved tabs ≠ N renderers at launch. **All** tabs persist (ephemeral included — user's call; ADR-0006
  placement semantics unchanged). App-quit teardown is masked (`tabs::begin_shutdown`) so draining
  browsers aren't persisted as user closes. **Verified:** `just verify` green (32 core tests: restore
  sanitization, session round-trip, shutdown mask); runtime with seeded session + isolated `$HOME` —
  restore→hydrate-active-only→load 200, title write-through, new-tab id continuation (core id 3 after
  restored 1–2), file intact through kill -9. **Pending interactive:** a by-hand ⌘W/⌘Q quit→relaunch
  cycle (the graceful-close mask is unit-tested; lldb can't drive CEF's close flow representatively) —
  rides the standing P0-1/M2 interactive pass.
- **Landed 2026-07-06 (P1-17) — reopen closed tab (⌘⇧T).** Reuses the ADR-0013 suspend/hydrate path:
  a single-tab close now records a `ClosedTab` snapshot (id, space/folder/pinned, url/title, the sibling
  it sat before) onto a capped, in-session stack in `TabStore` (`remove_tab_recording`); ⌘⇧T pops it and
  re-inserts a *suspended* tab into its original space/section/slot as the active tab (`reopen_closed`),
  and the host hydrates it on the resulting `StructureChanged` exactly like a session-restored tab — no
  new hydration path. Self-healing: a deleted space/folder falls back to the active space / loose-pinned;
  a moved/closed sibling lands the tab at its section end. Bulk removals (space delete/demote) use plain
  `remove_tab` and are deliberately *not* reopenable. The stack is **not** persisted (session restore
  already returns everything open at quit). New FFI `browser_tab_reopen_closed` + `browser_tab_can_reopen_closed`
  (menu-enabled state, like Back/Forward); File → "Reopen Closed Tab", disabled when empty. **Verified:**
  4 new core unit tests (in-slot restore, pinned/folder restore, space/folder-gone heal, stack cap) +
  `just lint` clean + `just build-macos` green. **Pending interactive:** the live ⌘⇧T keystroke→hydrate
  path rides the standing P0-1/M2 focus-routing pass (same menu key-equivalent mechanism as ⌘W, which is
  itself unverified-on-device).
- **Landed 2026-07-06 — Arc control placement + sidebar URL chip.** Controls reordered to Arc's layout
  (sidebar-toggle + back/forward left, reload right); a compact current-tab **URL chip** below the
  controls opens the command bar in **edit-in-place** mode (`Mode.editCurrent`, `Navigation.route`),
  wiring `Cmd+L` / File → "Open Location…". Reverses the guide's "no persistent address bar" over-reach
  — recorded in **ADR-0004**; style §3/§6/§13 updated. `just build-macos` green; behavior (select-all,
  navigate-current) verified by build, not yet by a running-app pass (needs P0-1 focus routing to
  daily-drive). Reconcile with M2 glass chrome when it lands. **Superseded 2026-07-06 by inline editing
  (below) — the chip no longer routes into the command bar.**
- **Landed 2026-07-06 — sidebar URL chip edits inline (supersedes ADR-0004).** Clicking the chip / `Cmd+L`
  now turns it into a live search/URL field *in the sidebar* with the autocomplete dropping down over the
  tab list, instead of opening the command bar edit-in-place. The command bar is now `Cmd+T` new-tab only
  (`Mode` removed). Extracted the shared field logic into `AutocompleteField`; new `URLBar` + `URLBarModel`;
  only the on-screen sidebar hosts the live field, reveal auto-dismiss suspended while editing. Recorded in
  **ADR-0014** (0004 flipped to Superseded); style §3/§6/§13 + parity updated. `just build-macos` green.
  **Pending interactive:** focus stealing from CEF, keyboard nav (arrows/return/escape), the
  dropdown-over-tab-list render, and click-away dismissal need a running-app pass (rides P0-1 focus routing).
- **Reconcile (2026-07-06):** the FFI + shell are **single-browser** today ("the primary browser" throughout
  `crates/ffi`). A fair amount of Phase-1-flavored UI (command bar ≈ M3, autocomplete, favicons) already
  exists against that single browser, but the **multi-tab spine (M1) is absent** — no `tabs` module, no
  `TabTitleChanged`-style event stream. M1 is the real structural unblock for M2–M5.
- **Landed 2026-07-07 (P1-8 + P1-9 → done) — file-monolith splits (`code-style.md` §3).** Decomposition-only,
  no behavior change. **P1-8:** `crates/core/src/client.rs` → `client/{mod,handlers,favicon,devtools}.rs` —
  handlers (display/life-span/load/find/focus/request + Alloy error pages + Views delegates), favicon download,
  DevTools/context-menu/F12 in separate files; `BrowserHandler` + `BrowserClient` stay in `mod.rs`.
  **P1-9:** `BrowserRootView.swift` → `Sidebar`, `TabRow`, `TabFavicon`, `EmptyStateView`, `FloatingSidebar`,
  `TrackingView`, `CEFContainerView`; `CommandBar.swift` → `CommandBarController`, `CommandBarPanel`,
  `CommandBarKeys`, `CommandBarView`. **Verified:** `just verify` green (162 core tests, clippy, fmt);
  `just build-macos` **BUILD SUCCEEDED**.
- **Landed 2026-07-07 (P1-21 → done) — URL chip neutralized while a surface is open (ADR-0022 follow-up).**
  When History/Passwords covers the canvas the sidebar chip no longer shows the active tab's host — it renders
  a muted, non-interactive label ("History" / "Passwords" + clock/key icon) with shield/vault/copy suppressed.
  `openHistory`/`openVault` call `URLBarModel.endEditing()`; `beginEditing` and ⌘L (`editCurrentURL`) no-op
  while `isSurfaceOpen`. Dismissing the surface restores the normal chip. **`just build-macos` green** before
  the parallel P1-8/P1-9 split landed; re-verify after those merges settle.
- **Landed 2026-07-06 (P1-10) — omnibox-grade autocomplete (ADR-0011).** Replaced the history
  `LIKE '%q%'` substring matcher with a purpose-built, dependency-free fuzzy scorer in the core
  (`crates/core/src/fuzzy.rs`, Smith-Waterman DP returning score + matched char indices) and a
  class-based ranking policy in `history::query` (host-prefix → host → URL → title, blended with
  log-scaled frecency; scans the 2000 most-frecent rows). Suggestion FFI JSON gained `displayUrl` +
  `titleMatches`/`urlMatches`. Swift: dropdown now **highlights** matched characters; the
  `SuggestionController` is **two-tier** (local providers instant, Google-suggest debounced) and
  publishes an **inline autocomplete** completion for the default host-prefix match; `CommandBarView`
  fills the field with the completion + selected tail, tracking typed-vs-field text and suppressing on
  delete. **Verified:** `just verify` green (core+ffi build/test/lint/fmt; 23 core tests incl. 8 fuzzy
  + 6 ranking); `just build-macos` green. **Interactive verification pending** for the inline-fill
  `TextField` selection UX (fill/select-tail/suppress-on-delete) — state-heavy SwiftUI that can't be
  unit-tested; rides the same running-app/focus-routing pass as P0-1 / M2.
- **Landed 2026-07-06 (P1-13) — History page polish.** The internal-page shell moved to the macOS 26
  toolbar idiom (style §15 updated): large title + Liquid Glass **search capsule** (`glassEffect`,
  matching the command bar) + glass Clear menu, floating over the list via `safeAreaBar` (system
  scroll-edge effect; no stacked `.bar` material — was flagged as glass-on-glass vs. §3/§13). Content:
  plain-style list, roomier rows (medium-weight titles, 18px favicons, hover highlight), restyled day
  headers. Day titles are **plain rows, not `Section` headers** — pinned plain-style headers draw a
  system hairline under themselves that neither `listRowSeparator` nor `listSectionSeparator` removes
  (user flagged it as a too-strong border); trading away header pinning removes it. **Verified** in the running app (isolated `$HOME`, seeded `history.sqlite`, window-float
  screenshots): header, capsules, grouping, favicons all render; the scroll-under edge effect itself
  couldn't be exercised (SwiftUI-native scrolling isn't reachable via `NSScrollView` from lldb, then
  the display slept) — it's the system behavior of `safeAreaBar`, low risk.
- **Landed 2026-07-07 (P0-3 + P1-18) — adblock end-to-end: network engine + shield UI (ADR-0020).**
  New `crates/core/src/adblock/` (engine/state/lists over Brave's `adblock` 0.12.5): a process-global
  `RwLock<Arc<Engine>>` built from a compiled-in **seed** at startup, then background-fetched
  EasyList+EasyPrivacy hot-swapped in; a `psl`-backed domain resolver (embedded resolver + `single-thread`
  off — the latter is what makes `Engine: Send+Sync` for the IO-thread + hot-swap sharing); persisted
  `adblock.json` (global switch + registrable-domain allowlist, mirrors `cdp.rs`); per-tab blocked counts.
  Request hook in `client.rs`: `wrap_request_handler!` → `wrap_resource_request_handler!` →
  `on_before_resource_load` returns `CANCEL` for blocked sub-resources (never the main frame), notes the
  block, maps CEF resource types → adblock strings; `adblock::init()` wired into `embed::initialize`; count
  reset on nav-start / forget on tab-close. FFI: `browser_adblock_{is_enabled,set_enabled,active_blocked_count,
  active_site_allowlisted,set_active_site_allowlisted}` (active-tab-scoped so no core-id↔CEF-id translation).
  Swift: a **shield** at the URL chip's trailing edge (filled+count when actively blocking, `shield.slash`
  when off) → native popover (count + per-site toggle); View-menu global + per-site toggles with
  `validateMenuItem` checkmarks/enablement; `BrowserState` mirrors the state, refreshed off load-start/end +
  tab-switch. **Verified:** `just verify`-equivalent green — 125 core tests (11 new: engine block/allow/
  exception/`$third-party`/fail-open, PSL resolver, allowlist add/remove + registrable-domain keying, state
  round-trip, per-tab counts), clippy `-D warnings` + fmt clean; **macOS app builds**; **runtime smoke
  (isolated `$HOME`): app starts with no panic and logs `[adblock] filter lists refreshed (3.5 MB)`** —
  seed→fetch→parse→hot-swap proven live. **Pending interactive:** a live-page pass to watch a real block +
  the shield count increment + the popover/menu toggle reload (the request-handler-fires-on-a-real-page
  integration point unit tests can't cover) — rides the standing M2 interactive pass. **Cosmetic
  (element-hiding) deferred to P2-21** (needs the P2-14 document-start substrate; load-end injection would
  FOUC — ADR-0020).
- **🎉 Landed 2026-07-07 (P2-21 → done) — cosmetic ad-block on the substrate (ADR-0020).** Element-hiding, the
  fast-follow to network blocking, now rides the P2-14 substrate as its **second production consumer** (alongside
  autofill — the live smoke logged `[inject] substrate installed tab=1 (2 consumer(s))`, the "one hard thing, many
  riders" payoff). New `adblock/cosmetic.rs`: a substrate consumer (channel `"cosmetic"`) whose document-start
  isolated-world script reports `location.href` and injects the returned CSS as a `<style>` on `documentElement`
  (tolerating a not-yet-existent `<head>`) — **before the page renders, so no FOUC** (the exact reason ADR-0020
  deferred cosmetic until the substrate existed). `adblock::cosmetic_css(url)` gates on the same global-switch +
  per-site allowlist as `check`, then `engine::cosmetic_css` folds the engine's `url_cosmetic_resources(url)`
  **`hide_selectors`** into one sorted `…{display:none!important}` rule. Registered unconditionally in
  `embed::initialize`. **Scope (deliberate, flagged):** plain site-specific `hide_selectors` only. **Follow-ups**
  (all filed here, not silent): `procedural_actions` (JSON `:has()`/`:style()` — need a JS operator), **generic**
  class/id cosmetics (need the page's observed class/id names via a `hidden_class_id_selectors` round-trip),
  **scriptlets** (`injected_script` — need **main-world** injection, a substrate capability the isolated-world
  bridge doesn't cover), and **sub-frame** hiding (needs the all-frames substrate, **P2-24**). **Residual flash
  risk:** the CSS arrives via an async (intra-process, sub-ms) round-trip at document-start — effectively
  flash-free for normal pages, but a 1-frame flash is possible on a trivially small/fast page. **Verified:** `just
  verify` green — 149 core tests (4 new: `cosmetic_css` builds the hide rule for a site-specific filter and is
  empty for other hosts; consumer reply-shape + inject-script shape); clippy `-D warnings` + fmt clean. **Live
  smoke (isolated `$HOME`, `BROWSER_ADBLOCK_DEMO` installs a stable seed+`localhost##.ad-banner`/`###ad-box`
  engine with no refresh, native-shell + CDP):** a page with `.ad-banner` + `#ad-box` + real content →
  `getComputedStyle().display === 'none'` for **both ads**, `block` for the real content (not over-hidden). Re-run:
  that env + `scratchpad/cosmetic_e2e.py`. **Next in the annoyances line:** P2-24 (all-frames substrate) then P2-22
  (cookie-consent, reject-first — ADR-0026).

## Phase 2 — Public Beta: "a complete browser" · *not current; partially itemized*

Goal (rm §5.2): a stranger switches without a week-one wall — the Minimum Viable Daily Driver (rm §4.1) — and
you can ship security patches. **Only on-request captures are itemized here**; the full §5.2 scope (import,
session restore, settings UI, vault, release infra, …) stays at phase granularity in `roadmap.md` until
Phase 1 nears exit.

| ID | Item | Status | Est | Deps | Refs |
|----|------|--------|-----|------|------|
| P2-1 | Tab taxonomy in core — a tab *type*: pinned (per-space, persistent) vs. today (ephemeral) | later | ~40k | P1-M1 | mvp §Arc-UX ("pinned/today split is a fast-follow"); user 2026-07-06 |
| P2-2 | Rename tab — double-click for inline rename + context-menu "Rename" | later | ~20k | P2-1 | user 2026-07-06 |
| P2-3 | Clear ephemeral (today) tabs — keep the active one; multi-select marks tabs to keep, "Clear" removes the rest | done | ~25k | P1-7 | user 2026-07-06 · landed in P1-23 2026-07-08 |
| P2-4 | Pinned-tab saved-URL semantics — navigate freely, "close" reverts to the saved URL (tab persists, reopenable), + "Replace URL with current" | later | ~30k | P2-1 | Arc pinned-tab behavior; user 2026-07-06 |
| P2-5 | Global pinned tabs (Favorites) — pinned across *all* spaces, distinct from the per-space persistent tabs | later | ~35k | P2-1 | user 2026-07-06 |
| P2-36 | **Pinned tabs** (Arc-like, distinct from saved) — always resident / preloaded in each space's sidebar, separate from the saved/open split; plus a setting for when to clear **open** tabs (never by default) | wip | ~50k | P1-23 | style §4; ADR-0032; user request 2026-07-08 |
| P2-37 | **Page-aware sidebar tint (optional)** — Settings → Appearance toggle (off by default): tint the sidebar background to the active tab's sampled page color (`theme_color.rs` → `TabThemeColorChanged`); updates on navigation/SPA changes, **no scroll-state gating** (unlike the viewport bar). `ThemeColor.swift` CSS parser + `SidebarPageTintBackground`. | done | ~20k | — | `Sidebar.swift` · `FloatingSidebar.swift` · `SettingsView.swift` · `ThemeColor.swift`; user request 2026-07-08 · landed 2026-07-08 |
| P2-6 | Downloads indicator + section (Tier-1 downloads manager) | later | ~40k | — | rm §4.1 / §5.2; user 2026-07-06 |
| P2-7 | Traffic-light native-parity polish — option-key alternate glyphs, VoiceOver labels/traits, green zoom-vs-fullscreen nuance (deferred from P1-14's custom-drawn lights) | later | ~20k | P1-14 | ADR-0012 |
| P2-8 | Vault crypto core — key hierarchy (KDF → master key → password hash → stretched key → user symmetric key) + Bitwarden `EncString` (AES-256-CBC + HMAC-SHA256), unit-tested in isolation | done | ~35k | — | ADR-0016 · spec §10; user 2026-07-06 |
| P2-9 | Vault auth/login — prelogin (KDF params) + token grant + 2FA + device id; first Bitwarden/Vaultwarden compat validation | done | ~50k | P2-8 | ADR-0016 · spec §10 |
| P2-10 | Vault sync + cipher model — `GET /api/sync`, decrypt ciphers into a login-item model | done | ~40k | P2-9 | ADR-0016 |
| P2-11 | `vault_lookup(host)` + lock/unlock state + encrypted local cache | done | ~35k | P2-10 | ADR-0016 · spec §9/§10 |
| P2-12 | Vault FFI surface (`browser_vault_*`) | done | ~25k | P2-11 | ADR-0016 · spec §5 |
| P2-13 | Native vault management UI — unlock, list/search, add/edit login, settings, + import-flow UI (file picker → preview → confirm) | wip | ~90k | P2-12 · P1-M2/M3 | ADR-0010/0016 · style |
| P2-14 | Content-script injection substrate — document-start, isolated world, page↔core message channel (CDP `addScriptToEvaluateOnNewDocument`); shared by autofill, cosmetic ad-block, userscripts | done | ~45k | P0-4 | ADR-0017 · spec §11/§20 |
| P2-15 | Vault autofill core — form detection + fill content script + `vault_lookup` matching + fill/save message protocol | done | ~55k | P2-11 · P2-14 | ADR-0017 · spec §10 |
| P2-16 | Autofill in-page inline picker — multi-match inline menu (isolated-world shadow-DOM, select-to-fill); supersedes P2-15's focus-auto-fill | done | ~35k | P2-15 · P2-12 | ADR-0017 · style |
| P2-17 | Credential importer (core) — parse generic CSV + Bitwarden JSON export → ciphers via `POST /api/ciphers/import` | done | ~35k | P2-10 | ADR-0016 · rm §5.2; user 2026-07-06 |
| P2-18 | Direct browser credential import — Chrome/Safari/Arc extraction (encrypted Login Data + OS keychain) | later | ~70k | P2-17 | rm §4.1/§5.2; deferred broad-import option (user 2026-07-06) |
| P2-19 | Full-URL `vault_lookup` fidelity — a lookup-by-URL overload so `Exact`/`StartsWith` match paths precisely + `RegEx` match support (a regex engine); today host-only input degrades these (ADR-0016) | later | ~25k | P2-11 | ADR-0016; discovered in P2-11 |
| P2-20 | Vault token secure-storage — persist access/refresh tokens in the macOS Keychain (core export/restore-session seam + Swift provider) so relaunch skips re-login; today the session is in-memory only (offline-unlock covers reads) | later | ~35k | P2-12 · P2-13 | ADR-0016; scoped out of P2-12 |
| P2-21 | Cosmetic ad-block (element-hiding CSS + scriptlets) on the document-start injection substrate — the fast-follow that completes adblock beyond network blocking | done | ~30k | P2-14 · P0-3 | ADR-0020 · spec §11; user request 2026-07-07 |
| P2-22 | Cookie-consent — reject-first CMP automation (Tier B core, ADR-0026): a substrate operator (`consent.rs`) detects known CMPs (OneTrust/Cookiebot/Didomi via a core-served ruleset) and clicks **reject-all**, with a `MutationObserver` for async-injected banners — opt out, not just hide. Broader coverage + Tier A hide split to P2-25/P2-26. | done | ~40k | P2-14 · P2-21 | ADR-0026 · rm §3.1 (Tier-2 annoyances); user request 2026-07-07 |
| P2-25 | Cookie-consent **Tier A** (hide) + a **separate cookie-consent toggle** governing the whole feature (reject + hide), independent of ad-block. Long-tail *hide* of consent banners via a curated static consent-plugin stylesheet (generic selectors; the full EasyList-Cookie-via-engine path is a follow-up — needs generic cosmetic matching). Honest fallback (hide ≠ reject; ADR-0026). | done | ~30k | P2-22 | ADR-0026; split from P2-22 2026-07-07; user chose separate toggle |
| P2-26 | Cookie-consent — **broaden reject coverage**: embed the full **Consent-O-Matic** (MIT) ruleset (many CMPs) + **hot-update** it like the adblock lists (bundle snapshot → background refresh → hot-swap); today a small bundled 3-CMP ruleset. Plus a **main-world** operator for TCF `__tcfapi` consent-string reject (needs the main-world-injection substrate capability), and cross-origin **iframe** CMPs via P2-24. | next | ~40k | P2-22 · P2-24 | ADR-0026 · Consent-O-Matic (MIT); split from P2-22 2026-07-07 |
| P2-23 | Autofill native save-password banner — detect a login-form submit with a new/changed credential (via the substrate) → offer to save it to the vault; **native** banner (Safari-style, not field-anchored ⇒ no coordinate mapping — ADR-0017). Split from P2-16 (the picker); **needs the M2 chrome** for the native surface. The detection + vault write-path underneath it is **P2-32** (core, unblocked); this row is the native confirm surface on top. | blocked | ~30k | P2-32 · P1-M2 | ADR-0017 · style; split from P2-16 2026-07-07 |
| P2-24 | Substrate — **cross-origin iframe (OOPIF) injection**. **Same-origin iframes already work** (free from P2-14 — verified). Cross-origin is **blocked by CEF's DevTools observer** (relays child-session *results* but not *events*, so `Target.setAutoAttach` can't carry the `bindingCalled` page→core edge — ADR-0027); the real fix is a **render-process handler** (per-renderer isolated-world injection + browser↔render IPC — ADR-0017's fallback), a distinct larger effort. Prereq for full cookie-consent (ADR-0026: iframe CMPs) + sub-frame cosmetic ad-block. | blocked | ~90k | P2-14 · render-process-handler architecture | ADR-0017/0026/0027; investigated 2026-07-07 |
| P2-27 | Vault **organization ciphers** — decrypt items shared via an org: unwrap the user's RSA private key (`profile.privateKey`, an `EncString` under the user key), RSA-decrypt each org's symmetric key (`profile.organizations[].key`, type-4 EncString), then decrypt org ciphers under it (respecting per-cipher `key`). Needs **type-4 (RSA-2048/OAEP-SHA1) EncString** support in `crypto.rs`. Today org ciphers are **skipped** in `parse_sync` (so one can't fail the whole sync). | next | ~40k | P2-10 | ADR-0016 · spec §10; discovered 2026-07-07 (per-cipher-key fix) |
| P2-28 | Autofill **resilient dynamic detection** — replace the single-shot `DOMContentLoaded` detector (`vault/autofill.rs` `DETECT_SCRIPT`) with a continuous one: a **`MutationObserver`** for forms injected after load, **SPA-navigation re-detection** (`pushState`/`replaceState`/`popstate` — same-document URL changes never re-run `addScriptToEvaluateOnNewDocument`), **shadow-DOM piercing** (walk open shadow roots when scanning for fields), all **debounced** so churn doesn't thrash. Fixes SPA sites + the **password screen of two-step logins** (Google/etc.). The `MutationObserver` refinement filed in P2-15's note. | next | ~35k | P2-15 | ADR-0017 · spec §10; user request 2026-07-07 |
| P2-29 | Autofill **two-step / username-first flows + `autocomplete`-driven field classification** — offer on a **lone identifier/email field** before any password field exists (today detection is gated on finding a password field, so Google's *first* screen offers nothing), and classify fields via the web-standard `autocomplete` tokens (`username`/`current-password`/`new-password`) + `type`/name/ARIA heuristics, covering **password-only** and **username-only** forms. Completes Google/Microsoft/Okta from screen one. | next | ~30k | P2-28 | ADR-0017 · spec §10; user request 2026-07-07 |
| P2-30 | Autofill **fill-event fidelity** — harden `setValue` to dispatch a fuller, framework-compatible sequence (`focus`→`keydown`→`beforeinput`→`input`→`keyup`→`change`→`blur`) via the native prototype value setter, so fills **stick** on React/Vue/Angular-controlled inputs and validation-gated forms (some reject the current `input`+`change`-only fill). Driven by observed real-site failures. | next | ~15k | P2-15 | ADR-0017; user request 2026-07-07 |
| P2-31 | **TOTP / one-time-code autofill** — core **RFC-6238** generator + `otpauth://` / raw-secret parse (surface the cipher's `totp`, today unused), and OTP-field detection in the substrate (`autocomplete="one-time-code"`, numeric `inputmode`, and the common **split N-box** OTP inputs) → fill the current code on the 2FA step. Ubiquitous situation; the vault already stores the secret. | next | ~35k | P2-28 · P2-11 | ADR-0017 · spec §10; user request 2026-07-07 |
| P2-32 | Autofill **save/update credential core** — substrate **submit-detection** (capture username+password on a login-form submit) → **new-vs-changed diff** against the vault → the vault **write path** (create/update cipher, encrypt under the user key, `POST`/`PUT /api/ciphers`, sync). Core is **CDP-verifiable ahead of the native banner** (like P2-15 preceded the picker); **unblocks P2-23** (the banner is the user-facing confirm on top). | next | ~45k | P2-15 · P2-10 | ADR-0017 · spec §10; user request 2026-07-07 |
| P2-33 | Autofill **password generation + new-credential fill** — a core strong-password generator + fill the **new-password/confirm** fields P2-29 classifies (signup + change-password forms), handing the result to P2-32's save path. The native-absorb answer to a password manager's generator. | next | ~30k | P2-29 · P2-32 | ADR-0017 · rm §3.1; user request 2026-07-07 |
| P2-34 | **Vault quick-access button + current-site logins popover** (Bitwarden/1Password toolbar-popup idiom) — a **native** key/vault button **beside the URL chip** (mirrors the P1-18 adblock shield: button + `.popover`), tinted/badged when `browser_vault_lookup(host)` has matches for the active tab. The popover lists this site's logins, each with **Fill**, **Copy username**, **Copy password** (reuse `VaultView`'s clipboard+checkmark rows); locked-state opens the existing unlock flow; empty-state offers search-all/add. The **detection-independent manual path** — Copy works on *any* page regardless of form detection (the reliable escape hatch when auto-fill can't find the fields, e.g. `google.com` two-step). **New capability (ADR-0029):** core-initiated **Fill** = `inject::evaluate_main_world` runs a lenient find-and-fill snippet (`autofill::fill_expression`) in the active tab's **main-frame main world** (`Runtime.evaluate`, no `contextId` — so no isolated-world context tracking; a fill exposes the value to the page regardless) + `browser_vault_fill_active_tab(id)` FFI (fetches the secret on the gesture). Reuses P2-12 FFI, P1-18 chrome, P2-14 substrate. Complements the auto-picker (P2-16/P2-28/P2-29): manual + deterministic vs. automatic-on-focus. | done | ~45k | P2-12 · P1-18 · P2-14 | ADR-0017/0029 · style §16/§17; user request 2026-07-07 |
| P2-35 | **Appearance — color scheme forwarding + Light/Dark/System setting** (ADR-0030). CEF's ALLOY runtime never propagated the OS appearance to web content, so `prefers-color-scheme` was stuck `light` (user-confirmed). Fixed by driving `CefRequestContext::SetChromeColorScheme` on the **global** request context (all tabs inherit; applies **live**, no reload — CDP-verified). Core (`theme.rs`) owns a persisted preference and resolves `System` from a **host-forwarded** OS appearance (KVO on `NSApp.effectiveAppearance`); the preference also drives `NSApp.appearance` so the **whole browser** (native chrome + web content) shares one theme, like Chrome/Arc. Surfaced as **View → Appearance → System/Light/Dark** (radio checkmarks); FFI `browser_theme_{preference,set_preference,set_system_dark}`. Verified end-to-end via the CDP harness: Light→light, Dark→dark, System→OS, and a menu change flips an already-open page live. | done | ~40k | — | ADR-0030 · style §11/§18; user request 2026-07-07 |

Notes:
- **🌗 Landed 2026-07-07 (P2-35 → done) — appearance / color-scheme forwarding (ADR-0030).** Root cause: the
  ALLOY runtime never wired Chromium's `NativeTheme` to the host, so web `prefers-color-scheme` was pinned
  `light`. Fix: `theme.rs` drives `set_chrome_color_scheme(LIGHT|DARK, 0)` on the **global request context**
  (`request_context_get_global_context()`) — one call themes every tab, and it propagates **live** to open
  pages (no reload). One preference (System/Light/Dark), persisted to `theme.json`, resolves `System` from a
  host-forwarded OS appearance and also sets `NSApp.appearance`, so chrome + content share one theme. Host
  seam: `AppDelegate` forwards `NSApp.effectiveAppearance` (initial + KVO) and owns the View → Appearance
  submenu; FFI `browser_theme_{preference,set_preference,set_system_dark}`. **Verified via CDP** (three isolated
  profiles Light/Dark/System on ports 9222–9224 + an AppleScript menu drive re-probed live): all four cases
  pass. Windows/Linux reuse the whole core surface — they add only their platform's appearance detection.
- **🔑 Landed 2026-07-07 (P2-34 → done) — vault quick-access button + current-site logins popover (ADR-0029).**
  A native **key button beside the URL chip** (`VaultKeyButton`/`VaultQuickAccess.swift`, mirroring the P1-18
  shield: button + `.popover`), accent-tinted when the active host has saved logins. The popover lists this
  site's logins — each row **Fills** (primary tap) with **Copy username** / **Copy password** beside it;
  locked → Unlock (opens Passwords), no matches → Open Passwords; copied passwords auto-clear after 30s. The
  **detection-independent** path: Copy works on any page; Fill lands even on a two-step form's email-only first
  screen (the `google.com` case). Wiring: `BrowserState.vaultMatches` refreshed off the event stream next to
  the shield (load start/end, tab switch) + on any vault lock-state change; `HostFavicon` consolidated to one
  shared view (was duplicated in History + Vault). **Fill mechanism (ADR-0029):** core-initiated **main-world**
  evaluate — `browser_vault_fill_active_tab(id)` → `embed::vault_fill_active_tab` fetches the credential on the
  gesture → `inject::evaluate_main_world` runs `autofill::fill_expression` (lenient find-and-fill) via
  `Runtime.evaluate` with no `contextId`. Rejected isolated-world + context-tracking as needless (a fill exposes
  the value anyway; main-frame only — the per-frame picker still covers sub-frames). **Verified:** `just verify`
  green (3 new core unit tests — main-world evaluate omits contextId, `fill_expression` escaping + leniency);
  macOS `BUILD SUCCEEDED`; **live end-to-end over CDP** (isolated `$HOME`, `BROWSER_VAULT_DEMO`, seeded session
  → localhost login form, `scratchpad/verify_fill.py`): the exact fill snippet fills email+password on a full
  form **and** the email on an email-only step-one page, no password field present — all PASS. The native
  button-render/click + popover interaction is standard SwiftUI on the proven state/FFI (not pixel-driven this
  pass — native chrome isn't CDP-drivable; an interactive click-through is the remaining confirmation).
- **🔑 Fixed 2026-07-07 (P2-10 completeness) — per-cipher key encryption.** A live bitwarden.com sync failed
  with `sync: crypto error: MAC verification failed`. Root cause: modern Bitwarden clients use **cipher key
  encryption** — each cipher carries its own `key` (a symmetric key, itself an `EncString` wrapped by the user
  key) and encrypts its fields with *that*, not the user key directly. `parse_sync` decrypted every field with
  the user key → guaranteed MAC mismatch on any cipher a recent client saved. The error was a *decryption* bug,
  not a connectivity one: login already MAC-verifies the user key when it unwraps the protected key
  (`auth.rs` `unwrap_key`), so reaching sync proves KDF/password/email/token/2FA all worked. **Fix:** `RawCipher`
  now reads `key`; `parse_sync` unwraps it per cipher and decrypts that cipher's fields with the per-cipher key,
  else falls back to the user key (folders stay under the user key). Org ciphers (`organizationId` present) are
  skipped pending **P2-27** rather than reproducing the same opaque failure. Two new unit tests build a genuine
  per-cipher-key `EncString` and assert the org-skip. `cargo test -p browser-core` green, clippy/fmt clean.
  **Live confirmation** (real vault decrypts end-to-end) is the user re-running "Set Up Passwords".
- **Cookie-consent suppression (P2-22) filed 2026-07-07 (user).** Clarified scope: the *EU consent
  banners/popups*, not HTTP `Set-Cookie` handling. It's a special case of cosmetic filtering, so it rides the
  same machinery already committed — the `adblock` engine (does element-hiding, not just network blocking) on
  the P2-14 document-start injection substrate — which is exactly why it lands **after** P2-14/P2-21, not now
  (load-end injection would flash the banner; ADR-0020's deferral rationale applies verbatim). Two tiers,
  intentionally split: **hide ≠ reject** — Tier A cosmetically removes the banner (cheap, folds into P2-21's
  list path) but leaves consent unanswered; Tier B embeds Consent-O-Matic to programmatically choose "reject
  all" at the CMP (the privacy-real behavior, extra per-CMP rule engine + maintenance). No Rust crate exists
  for Tier B — the logic is inherently page-DOM JS, so it's an embedded ruleset + injected operator, not a
  `cargo add`. Recommendation on record: ship both layered (matches the "no extensions — it's built in" pitch,
  rm §3.1). Est ~55k is the new work **beyond** P2-21 (Tier A ≈ list-wiring increment; Tier B ≈ ~40–60k).
- **Posture resolved 2026-07-07 (user "do it") — reject-first, hide as an honest fallback (ADR-0026).** The
  hide-vs-reject fork is decided: **reject-first** (opt out where a CMP is recognized), hide only as the long-tail
  fallback, and the product is honest that *hidden ≠ rejected* — because hide-alone leaves the user tracked behind
  a privacy-clean facade, which contradicts the trust thesis (rm §1.5/§3.1). Layer them (hide the visual at
  document-start + reject underneath) for a clean UI *and* a real opt-out with no flash. Mechanism: embed
  **Consent-O-Matic** (MIT) via the substrate, prefer **TCF consent-string** reject (`__tcfapi`) over DOM-clicking
  for IAB-TCF CMPs, hot-update the ruleset like the adblock lists. **Discovered prereq → P2-24:** Tier B must reach
  CMP dialogs in **iframes**, so the substrate needs **all-frames** injection (today main-frame only). Sequencing
  confirmed **P2-21 → P2-22**; P2-22's first cut is Tier B top-CMPs + Tier A underneath. Full rationale + rejected
  alternatives in **ADR-0026**.
- **🎉 Landed 2026-07-07 (P2-22 → done) — cookie-consent reject-first (ADR-0026).** The reject vertical is live: the
  substrate's **third production consumer** (`[inject] substrate installed tab=1 (3 consumer(s))` — autofill +
  cosmetic + consent). New `crates/core/src/consent.rs`: the core serves a **per-CMP ruleset** (`{present, reject}`
  selectors for OneTrust/Cookiebot/Didomi) over the `"consent"` channel; the isolated-world operator fetches it,
  finds a present CMP, and **clicks its reject-all control** — a real click on the page's button, so the CMP's own
  handler runs (consent *denied*, banner removed), not merely hidden. A `MutationObserver` (10 s, then disconnects)
  catches **async-injected** banners; each CMP is rejected once (`handled` set); rejects are reported back and
  logged (`[consent] rejected <cmp> on <host>`). **Precise selectors only — no generic reject-text matching** (that
  mis-clicks; precision is Consent-O-Matic's whole point). Registered unconditionally in `embed::initialize`.
  **Verified:** `just verify` green — 153 core tests (4 new: ruleset parses + lists the CMP rules; handler shapes;
  operator is isolated/precise/observing); clippy `-D warnings` + fmt clean. **Live smoke (isolated `$HOME`,
  `cdp.json`, native-shell + CDP, a mock CMP page):** a Cookiebot-shaped banner present at load **and** a
  OneTrust-shaped banner injected 500 ms later (exercising the observer) — the operator clicked reject on **both**
  (each page reject-handler set its main-world flag + dismissed its banner), `[consent] rejected cookiebot`/`onetrust`
  logged. **Bug caught + fixed live:** the first operator stopped after the first reject (never installed the
  observer), so the async banner was missed — fixed to always observe + reject-each-CMP-once. Re-run:
  `scratchpad/consent_e2e.py`. **Follow-ups (filed, not silent):** **P2-25** Tier A hide (long-tail, no-reject-rule
  banners); **P2-26** full Consent-O-Matic ruleset + hot-update + main-world TCF `__tcfapi` reject; cross-origin
  iframe CMPs ride **P2-24** (all-frames substrate — this reaches the main frame + same-origin frames only).
- **P2-24 investigated 2026-07-07 → blocked by a CEF limitation (ADR-0027); same-origin iframes already work.**
  Set out to add all-frames injection via `Target.setAutoAttach`. **Finding, verified live:** the substrate
  *already* covers the **main frame + same-origin (same-process) iframes** for free (a same-origin child iframe's
  `.ad-banner` is cosmetically hidden — confirmed via CDP `contentDocument`). **Cross-origin iframes (OOPIFs) are
  blocked:** a three-step probe showed `Target.setAutoAttach{flatten}` auto-attaches the OOPIF and sessionId-tagged
  messages reach it (results return), **but CEF's DevTools observer relays child-session *results*, not *events*** —
  a deliberate child `console.log` produced its result but **no `consoleAPICalled`/`executionContextCreated`/
  `bindingCalled` event**. The substrate's page→core edge *is* an event, so it can't arrive from an OOPIF. The
  non-working auto-attach code was **reverted** (no dead code); the finding + the real fix (a **render-process
  handler** — per-renderer injection + browser↔render IPC, ADR-0017's fallback) are recorded in **ADR-0027**, and
  P2-24 is reframed to that (blocked on the render-process architecture). `just verify` green — 153 core tests
  (substrate back to its proven state).
- **🎉 Landed 2026-07-07 (P2-25 → done) — cookie-consent Tier A (hide) + a separate toggle (ADR-0026, user request).**
  Cookie-consent is now a **user-controllable feature with its own toggle**, independent of ad-block, governing
  **both** tiers. `consent.rs`: a persisted `enabled` state (`consent.json`, default on — reject-first is the
  product default) + `is_enabled`/`set_enabled`/`load_persisted`; the reject ruleset + a **Tier-A hide stylesheet**
  are served in one **gated `config` message** (off ⇒ `{enabled:false}` and the operator does nothing). **Tier A**
  is a curated, conservative **consent-plugin stylesheet** (`#cookie-law-info-bar`, `.osano-cm-window`,
  `#usercentrics-root`, `[id^=sp_message_container_]`, … — plugin-specific ⇒ low false-positive) injected at
  document-start to hide the long-tail banners we have no reject rule for. **Hide + reject compose**: a
  programmatic `.click()` fires even on a button our CSS has hidden, so a CMP we both hide *and* reject is cleaned
  instantly *and* opted out underneath (proven live). Threaded core→FFI→menu: `embed::consent_set_enabled`
  (+reload), `browser_consent_{is_enabled,set_enabled}` FFI, and a **View → "Block Cookie Notices"** item
  (`BrowserState.consentEnabled` + `toggleCookieConsent` + `validateMenuItem` checkmark), mirroring the ad-block
  toggle. **Verified:** `just verify` green — 157 core tests (4 new: config gated by the toggle, ruleset shape,
  hide-CSS is plugin-specific/not over-broad, state round-trip) — + `just build-macos` **BUILD SUCCEEDED** (menu
  wiring compiles). **Live smoke (both toggle states, native-shell + CDP, a page with `#cookie-law-info-bar` +
  a OneTrust banner):** **on** → the plugin banner is `display:none` and OneTrust is **rejected** (its reject
  button hidden by our CSS, clicked anyway) + real content untouched; **off** (via `consent.json`) → banner
  **visible**, **nothing rejected** (0 `[consent]` logs) — the toggle gates the whole feature. Re-run:
  `scratchpad/p25_e2e.py {on|off}`. **Honest labeling** (hide ≠ reject) is internal for now — a user-facing
  "hidden vs rejected" indicator awaits a consent UI. **Follow-up = P2-26** (full EasyList-Cookie-via-engine +
  hot-update + generic cosmetic matching + main-world TCF; cross-origin iframe CMPs ride P2-24/ADR-0027).
- **Vault (P2-8..P2-13) — fork resolved 2026-07-06 (ADR-0016).** The Phase-2 credentials fork (`rm §5.2`)
  is decided: a **Bitwarden-compatible, rbw-derived** client (`spec §10`), **pulled forward** ahead of the
  remaining MVP1 shell polish at the user's call. Built **core-first, crypto-first** — the crypto is the
  correctness-critical, fully-isolable piece, so it lands and is unit-tested before any network/UI code
  leans on it. P2-8..P2-12 are frontend-agnostic Rust in `crates/core`; **P2-13 (autofill UI) rejoins the
  shell sequence** — it depends on the command bar (M3) + proven focus routing (P0-1). End-to-end
  Bitwarden-server compatibility (byte-exact KDF salts / HKDF info / EncString framing) is validated in
  **P2-9** against a live Vaultwarden — the crypto slice proves self-consistency + tamper detection +
  hierarchy wiring, not wire-compat (ADR-0016 records this boundary explicitly).
- **Landed 2026-07-06 (P2-8) — vault crypto core.** New `crates/core/src/vault/{mod,crypto}.rs`
  (frontend-agnostic, no CEF/network/UI): `Kdf` (PBKDF2-SHA256 + Argon2id), `MasterKey`
  (`derive` / `password_hash` / `stretch`), `SymmetricKey` (`from_bytes` / `encrypt` / `decrypt` /
  `unwrap_key`), and the type-2 `EncString` (`parse` / `Display` / `FromStr`). RustCrypto deps added to
  the workspace; key material is `zeroize`-on-drop. **Verified:** `just verify` green — 16 new unit tests
  (EncString round-trip / tamper / malformed-parse / bad-lengths, KDF determinism+salting, Argon2≠PBKDF2,
  password-hash stability, stretched-key divergence, full derive→stretch→wrap→unwrap→decrypt hierarchy)
  atop the existing 40 core tests; clippy `-D warnings` + fmt-check clean. **Next:** P2-9 (auth/login),
  which is where Bitwarden/Vaultwarden wire-compat gets validated end-to-end.
- **Landed 2026-07-06 (P2-9) — vault auth/login.** New `crates/core/src/vault/{http,auth}.rs`. `http.rs`
  is the transport seam: `HttpTransport` trait + a single blocking `ureq` (rustls) impl, configured to
  return 4xx bodies (a 2FA challenge is HTTP 400 with a body to parse) — the seam that makes the flow
  mock-testable. `auth.rs` is the Bitwarden flow: `ServerConfig` (cloud vs. self-hosted URL derivation),
  prelogin → `Kdf`, password **and** API-key (`client_credentials`) grants, `TwoFactor` (TOTP/YubiKey)
  resubmit, refresh, a random UUIDv4 device id (no `uuid` dep), and — the tie-in — `login` derives the
  master key (P2-8), sends the hash, then **decrypts the protected vault key into an unlocked `Session`**.
  Deps: `ureq`, `form_urlencoded` (the base64 password hash needs correct `+`/`/`/`=` escaping). Request
  build / response parse / error map are pure functions. **Verified in isolation:** `just verify` green —
  32 vault tests total (16 new auth: URL derivation, prelogin PBKDF2/Argon2 + PascalCase, form-field +
  header assertions incl. escaping + `Auth-Email`, 2FA detection in both body shapes, invalid-creds
  mapping, provider int round-trip, UUID shape, **and a full prelogin→token→decrypt round-trip + a
  wrong-password-fails-at-unwrap case whose `Key` fixtures are generated by the real crypto**); clippy
  `-D warnings` + fmt-check clean. **Pending live validation (the one thing isolation tests can't prove):**
  byte-exact wire-compat — endpoint paths, field casing, `client_id`/`deviceType` acceptance — against a
  **real Bitwarden/Vaultwarden**. The server-facing constants are centralized at the top of `auth.rs` for
  exactly this tuning; validation needs a run against a live Vaultwarden (user action / a future session
  with a test server). **Next:** P2-10 (sync + cipher model).
- **Vault scope expanded 2026-07-06 (user) — full UI + import + autofill; two forks decided.** The vault's
  frontend-facing completion decomposes as: **autofill** (P2-14 injection substrate → P2-15 core → P2-16
  picker/banner), **import** (P2-17 core → P2-13 import-flow UI; P2-18 direct-browser import deferred), and
  the **management UI** (P2-13). **Fork A — autofill picker = in-page inline menu** (not a native popover):
  cross-platform, no web-view→screen coordinate mapping, 1Password-style; save prompt stays native
  (ADR-0017). **Fork B — import starts file-based** (generic CSV + Bitwarden JSON); direct Chrome/Safari/Arc
  extraction is filed as the larger P2-18. **Sequencing:** the split is core-buildable-now (P2-10 sync,
  P2-17 importer, P2-15 autofill detection+matching — all unit-testable without a server or shell) vs.
  shell-gated (P2-12 FFI, P2-13/P2-16 UI — ride the M2/M3 chrome, itself blocked on P0-1 focus routing).
  Autofill's injection substrate (P2-14) is **shared** with cosmetic ad-block + userscripts (ADR-0017) —
  built once. Continuing core-first.
- **Landed 2026-07-06 (P2-10) — vault sync + cipher model.** New `crates/core/src/vault/sync.rs`:
  `GET {api}/sync` with the session bearer → decrypt **login** ciphers (type 1; cards/notes/identities
  skipped) into a plain `Vault { logins: Vec<Login> }` (`Login` = id/name/username/password/uris/totp +
  resolved folder name). Pure `parse_sync(body, &user_key)` (JSON + per-field EncString decrypt, null/empty
  → None, PascalCase aliases) is split from `VaultClient::sync` (the only network touch; 401 → `Unauthorized`,
  other non-2xx → `Server`). Added `HttpRequest::get`; `VaultClient` fields are `pub(crate)` so `sync` lives
  beside its model. **Verified in isolation:** `just verify` green — 8 new tests (full login+folder+uris
  decrypt with server-side fixtures built by the real crypto, non-login skip, null/PascalCase, wrong-key →
  Crypto, empty vault, and `VaultClient::sync` bearer header + 401/5xx mapping over a mock); 80 core tests
  total; clippy `-D warnings` + fmt-check clean. Same live-server caveat as P2-9 (endpoint/casing on a real
  Vaultwarden). **Next:** P2-17 (importer) or P2-11 (`vault_lookup` + lock state) — both core, buildable now.
- **Landed 2026-07-07 (P2-11) — `vault_lookup` + lock/unlock state + encrypted cache.** Three new modules.
  `lookup.rs`: host→login matching honoring Bitwarden's per-URI rule, **best-first** (exact host ranks above
  domain). Correct **registrable-domain** matching via the `psl` crate (Public Suffix List compiled in) — so
  `google.com` creds reach `accounts.google.com` but **never leak across a public suffix** (`a.co.uk` ≠
  `b.co.uk`, the footgun a naïve last-two-labels matcher would hit). `Never` excluded; `RegEx` fails closed;
  `Exact`/`StartsWith` degrade to exact-host given host-only input (full-URL fidelity filed as a follow-up,
  not silent). Enriched the P2-10 `Login` model with per-URI `match_type` (`LoginUri`/`UriMatch`). `cache.rs`:
  encrypted-at-rest snapshot (`vault.json`, atomic write-through like `session.rs`) holding only email + KDF +
  the protected key + the raw `/sync` blob — **nothing readable without the master password**; tokens
  deliberately **not** cached (they want OS secure storage, wired in P2-12). `manager.rs`: the `VaultManager`
  state machine — `unlock_online` (login→sync→decrypt→cache), `unlock_offline` (re-derive from cache, no
  network), `lock` (drops the `ZeroizeOnDrop` key), `lookup`, `resync`. Small enablers: `Kdf` is now
  Serialize; `Session` carries email + protected_key; `sync_raw` split out. **Verified:** `just verify` green
  — 56 vault tests (incl. the PSL leak-guard, the full online-unlock→lookup→lock cycle, and an **offline
  unlock from a cache written by a prior online unlock**, all with real-crypto fixtures); 96 core tests total;
  clippy `-D warnings` + fmt clean. **Follow-ups filed:** full-URL match fidelity + RegEx (P2-19); token
  secure-storage (P2-20). **Next:** P2-17 (importer) — last core piece before shell-gated
  work; then a live-Vaultwarden validation pass would de-risk P2-9/P2-10/P2-11 together.
- **Landed 2026-07-07 (P2-17) — credential importer (core).** New `import.rs`: header-driven **CSV** parser
  (accepts Chrome/Edge/Firefox/Safari/Bitwarden layouts by column name) + **Bitwarden JSON** export parser →
  normalized `ImportRecord`s, then encrypt-with-user-key + bulk `POST /ciphers/import`. Uses the `csv` crate
  for RFC 4180 correctness (a password with a comma/quote/newline round-trips — proven by test; a naïve split
  would corrupt it). Pure parse + `build_import_request` (serde-built body, per-field EncString) split from
  the networked `VaultClient::import`; `VaultManager::import` requires an unlocked session, uploads, then
  resyncs. **Scope cut (flagged):** folders are not preserved — everything imports to root (a follow-up; the
  value is getting creds in). **Verified:** `just verify` green — 10 new tests (CSV specials/quoting, Bitwarden
  + Safari headers, credential-less rows skipped, name-from-URL, empty-file error, Bitwarden JSON logins-only,
  a build-request round-trip that **decrypts every field back**, client 200/401, manager unlock-gated count);
  105 core tests total; clippy `-D warnings` + fmt clean. Same live-Vaultwarden caveat (endpoint/casing).
- **Milestone 2026-07-07 — the frontend-agnostic vault core is complete.** crypto → auth/unlock → sync →
  lookup + lock-state + cache → import, all in `crates/core`, all unit-tested without a server or UI (105
  core tests). Everything remaining is **shell-gated**: FFI (P2-12), management UI (P2-13), autofill
  substrate/core/picker (P2-14/15/16). Recommended gate before building on top: **one live-Vaultwarden
  validation pass** for P2-9/P2-10/P2-11/P2-17 together (the wire-compat the isolation tests can't prove).
- **Live validation PASSED 2026-07-07 — the wire-compat caveat on P2-9/P2-10/P2-11/P2-17 is now closed.**
  Ran a full **register → login → import → sync → lookup** round-trip against a real **Vaultwarden 1.36.0**
  (Apple `container` runtime). The vault code — prelogin/token/decrypt (P2-9), `/sync` decrypt (P2-10),
  `vault_lookup` (P2-11), and encrypt+`/ciphers/import` (P2-17) — **worked first try** against the live
  server, including a comma-bearing password surviving the CSV→import→sync round-trip and `vault_lookup`
  matching `example.com`/`github.com`. **One wire finding, in test scaffolding only:** Vaultwarden 1.36 uses
  Bitwarden's newer **two-step registration** (`POST /identity/accounts/register/send-verification-email` →
  token, then `/identity/accounts/register/finish` with an RSA keypair) — registration is not a vault feature
  we build, so no core change. **Deliverable:** `crates/core/tests/vault_live.rs` — a self-contained,
  `#[ignore]`d, `live-tests`-feature-gated harness (self-registers a fresh account, exercises the public API),
  reusable on the security treadmill. Run: `VAULT_TEST_URL=http://<server> cargo test -p browser-core
  --features live-tests --test vault_live -- --ignored`. `just verify` stays lean — the RSA dep only compiles
  under the feature (verified: 105 core tests, clippy + fmt clean, no `rsa` in the default build).
- **Landed 2026-07-07 (P2-12) — vault FFI surface.** New `vault/service.rs`: a process-global
  `VaultManager<UreqTransport>` (like `embed`/`tabs` own the browser) behind a flat, FFI-friendly API — `init`
  (persists a stable device id + points at the cache), `is_locked`, `unlock_online` (optional TOTP) /
  `unlock_offline`, `lock`, `lookup`/`list` (→ `LoginView`, **metadata only, no password**), `password_for`
  (the single path a secret crosses, on explicit gesture — ADR-0017), `import_csv`/`import_bitwarden_json`.
  New `manager::logins()` exposes the full list for the UI. FFI (`crates/ffi`): 11 `browser_vault_*` C
  functions — thin glue, JSON-envelope returns (`{"ok":…}`/`{"error":…}`), strings freed via the existing
  `browser_string_free`; networked calls documented **off-main-thread** (they hold the global lock while
  blocking — a non-blocking callback variant is a noted refinement). **Verified:** `just verify` green — 3 new
  service tests (LoginView drops the password, device-id persist/reload, before-init safe defaults) + FFI
  builds; 108 core tests; clippy `-D warnings` + fmt clean. The service is thin glue over the
  **live-validated** client, so no new wire surface. **Scoped out (→ P2-20):** token secure-storage — the
  session is in-memory only; relaunch re-logins to sync, offline-unlock covers reads. **Next:** the vault core
  + FFI are complete; remaining vault work (P2-13 UI, P2-14/15/16 autofill, P2-20 tokens) is shell-gated on
  M2/M3 (blocked on P0-1 focus routing).
- **🎉 Autofill works END-TO-END in the running browser 2026-07-07 (v0).** New `vault/autofill.rs` +
  `client.rs` `on_load_end` hook: on a **main-frame** load the core takes the page host, `vault_lookup`s the
  unlocked vault, and — on a match — injects a content script (`frame.execute_java_script`) that finds the
  username/password fields and fills them (native value setter + `input`/`change` events for framework
  inputs). **Verified live** (not just unit tests): launched the app bundle under an isolated `$HOME` with
  CDP enabled (P0-4) + a debug demo vault (`BROWSER_VAULT_DEMO`, `service::seed_demo_vault`), drove it over
  CDP (`scratchpad/autofill_e2e.py`) to a local login page, and **read back both fields correctly filled**
  from the vault credential (`[autofill] injected fill for localhost`). `just verify` green — 114 core tests
  (2 new autofill), clippy + fmt clean. **This is a deliberate v0** (proves the chain), *not* the ADR-0017
  production model: it is one-directional core-push, runs in the page's **main world**, and **auto-fills
  without a picker or user gesture**. The production model remains: **P2-14** (isolated-world injection
  substrate via `ExecuteDevToolsMethod`), **P2-16** (in-page picker + save banner + gesture-gating). Those
  three (P2-14/15/16) stay open as the productionization; this v0 de-risks the whole approach and confirms the
  `cef` crate exposes everything needed (`execute_java_script`, `execute_dev_tools_method`,
  `add_dev_tools_message_observer`).
- **🎉 Landed 2026-07-07 (P2-14 → done) — content-script injection substrate (ADR-0017).** The *one hard thing
  built once* that autofill (P2-15/16), cosmetic ad-block (P2-21), and userscripts/Tweaks (spec §20) all ride.
  New `crates/core/src/inject.rs`: a consumer registry (`register({channel, script, handler})`) + a per-browser
  install that drives CEF's **in-process** DevTools (`BrowserHost::send_dev_tools_message` + an observer via
  `add_dev_tools_message_observer` — **no external CDP port**, ADR-0017's internal path). Per browser it sends
  `Page.enable`/`Runtime.enable`, `Runtime.addBinding{name, executionContextName}` (exposes `window.__browser_core_send`
  **only in the isolated world** — the page→core edge), and `Page.addScriptToEvaluateOnNewDocument{source, worldName,
  runImmediately}` for a bootstrap + each consumer script, at **document-start in a named isolated world**. The
  bootstrap installs `window.__browserCore.send(channel,data) → Promise` / `__receive`. **Reply routing needs no
  context tracking:** `Runtime.bindingCalled` carries the originating `executionContextId`, so the core→page reply
  is a `Runtime.evaluate{contextId}` into that exact isolated context (request/response). **Design choices:**
  `send_dev_tools_message` (raw serde-built JSON) over `execute_dev_tools_method` (avoids `DictionaryValue`
  marshalling); `&'static str` consumer scripts (generic source only — **secrets cross only via the channel**, the
  fix for the v0's main-world/password-baked-in flaw); **install is a no-op with zero consumers**, so a release build
  pays nothing per page until a rider registers (autofill migration = P2-15). Wired: `lib.rs` `pub mod inject`,
  `client.rs` life-span handler (`install_for_browser` in `on_after_created` before first doc; `uninstall_for_browser`
  in `on_before_close`), `embed::initialize` registers a debug demo consumer behind `BROWSER_INJECT_DEMO`. **Verified:**
  `just verify` green — 140 core tests (8 new: binding-called/payload parsing, channel dispatch, JS-literal reply
  escaping, the CDP message builders, a full page→core→page round-trip), clippy `-D warnings` + fmt clean. **Live
  smoke (isolated `$HOME`, `cdp.json` enabled, `BROWSER_INJECT_DEMO=1`, native-shell bundle):** drove a CDP navigation
  and read the **shared-DOM marker** the isolated-world script set after receiving the core's reply →
  `data-browser-inject-demo=ok:core-saw:from-isolated` (proves inject + page→core + core→page), while
  `window.__browserCore` and `__browser_core_send` are **`undefined` in the main world** (proves isolation); core log
  shows `[inject] substrate installed tab=1 (1 consumer(s))`. Re-run: launch that env + `scratchpad/inject_e2e.py`.
  **Timing note (for P2-15):** install runs in `on_after_created`, so a tab's *very first* document can race the
  addScript registration; user/CDP navigations after that are covered — autofill must handle the first-load case
  (re-inject on the initial `on_load_end`, or accept fill-on-next-nav). **Next:** P2-15 migrates autofill onto the
  substrate (isolated world + gesture), retiring the v0 main-world push.
- **🎉 Landed 2026-07-07 (P2-15 → done) — vault autofill on the substrate (ADR-0017).** The v0 (main-world,
  password-baked-in `on_load_end` push) is **retired** and replaced by a proper isolated-world content script that
  *asks the core*. `vault/autofill.rs` rewritten as a **substrate consumer** (channel `"autofill"`): a generic
  `DETECT_SCRIPT` runs at document-start in the isolated world, defers form detection to `DOMContentLoaded`
  (password field + preceding visible username field), and speaks a two-message protocol to the core —
  `{type:"lookup", host}` → `{matches:[{id, username}]}` (**metadata only, never a password** — `service::lookup`)
  and `{type:"password", id}` → `{password}` (the **single seam a secret crosses**, `service::password_for`). The
  script requests the password **only on a real user gesture** — field `focusin` guarded by
  `navigator.userActivation.hasBeenActive`, so a page's autofocus can't pull a password out — then fills via the
  native value setter + `input`/`change` events. Registered unconditionally in `embed::initialize` (the substrate's
  first production rider; `client.rs` `on_load_end` autofill block removed). **Scope (deliberate, flagged):** fills
  the **best single match**; the multi-match **in-page picker** + the **save-password banner** are **P2-16** (need
  M2 chrome). **SPA/dynamically-injected forms** (appearing after `DOMContentLoaded`) aren't detected yet — a
  `MutationObserver` refinement, filed here, not silent. **First-load race** (P2-14 note) accepted as
  fill-on-next-nav for a fresh browser's very first document. **Verified:** `just verify` green — 145 core tests
  (6 new: metadata-only lookup reply carries no password key, null-password reply resolves rather than hangs,
  locked-vault → empty matches / null password, unknown-type → no reply, detect-script is isolated + gesture-gated +
  DOM-ready), clippy `-D warnings` + fmt clean. **Live smoke (isolated `$HOME`, `cdp.json`, `BROWSER_VAULT_DEMO`,
  a localhost login form, native-shell bundle):** drove it over CDP — the password field is **empty after load**
  (gesture-gating proven), and after a **real mouse click** into the field it fills `octocat@example.com` +
  `correct horse battery staple` from the demo vault (detect → lookup → gesture → password → fill, all via the
  isolated-world substrate); logs show `[inject] substrate installed tab=1 (1 consumer(s))`. Re-run: that env +
  `scratchpad/autofill_e2e.py`. **Next:** P2-16 (in-page picker for multi-match + native save-password banner) —
  now unblocked by P2-14/P2-15 (still gated on M2 chrome for the native banner).
- **🎉 Landed 2026-07-07 (P2-16 → done) — in-page autofill picker (ADR-0017); save banner split to P2-23.** The
  ADR-0017 **picker** — an in-page inline menu the content script draws (chosen over a native popover: no
  web-view→screen coordinate mapping, works on every frontend). All in `vault/autofill.rs`'s `DETECT_SCRIPT`, no
  Rust/native change beyond adding `name` to the lookup metadata: on gesture-focus of a login field the isolated-world
  script opens a menu in an **open shadow root** (so the page's CSS can't restyle it — the ADR's "DOM chrome we own"),
  lists the matching logins (username + login name), supports **keyboard (↑/↓/↵/esc) + click + hover**, repositions on
  scroll/resize, flips above the field when there's no room below, and dismisses on outside-pointerdown/escape. Vault
  data is written via `textContent` (never `innerHTML`). **This supersedes P2-15's interim focus-auto-fill:** focus now
  *shows* the menu (metadata only); the password crosses **only when the user selects an entry** — a stronger, explicit
  gesture, still the single `password_for` seam. **The native save-password banner is split out to P2-23** (blocked on
  P1-M2 — it needs the native chrome; not field-anchored, so no coordinate mapping, ADR-0017). **Verified:** `just
  verify` green — 145 core tests (autofill tests updated: metadata now carries `name`, and a guard that the password
  request lives in the *selection* path, not focus; `textContent` not `innerHTML`); clippy + fmt clean. **Live smoke
  (2-login demo vault for `localhost`, native-shell + CDP):** focus → the picker shows **both** matches with the
  password field **empty** (no fill on show), and clicking the **second** entry (`work@example.com`) fills **that**
  credential (`hunter2-work-pw`, not the other) — proving multi-match render, gesture-gated metadata-first, and correct
  selection routing. **Screenshotted** the rendered menu (`scratchpad/picker.png`) — a clean 1Password-style card
  (rounded, shadowed, selected row in macOS blue, username over login name). Re-run: `scratchpad/picker_e2e.py`.
  `seed_demo_vault` now seeds two localhost logins for this. **Autofill is now the full ADR-0017 model** (isolated-world
  detect → picker → gesture-select → fill); the only remaining autofill piece is the native save banner (P2-23/M2).
- **P0-4 CDP surface — verified live 2026-07-07.** The "pending running-browser check" is done: launched with
  `cdp.json` `{"enabled":true}` under an isolated `$HOME`; `curl http://127.0.0.1:9222/json/version` returned
  the CDP target list and `DevTools listening on ws://127.0.0.1:9222`. **Wire note for CDP clients:** Chromium
  rejects websocket connections that send a disallowed `Origin` header (403) — native clients must omit it
  (`suppress_origin`), or the surface needs `--remote-allow-origins`; adding that flag to `cdp::switches` for
  broader tool compatibility is a small follow-up if real automation tools (which send `Origin`) need it.
- **🎉 Vault UI works END-TO-END in the running app 2026-07-07 (P2-13 v0).** New
  `apps/macos/Sources/VaultView.swift` — the native Passwords surface (ADR-0010 content-area surface, mirrors
  History): unlock screen (offline, master password), searchable **login list**, and **on-gesture password
  reveal/copy** (the only path a secret crosses the FFI — `browser_vault_password`, ADR-0017). Bridged via the
  11 `browser_vault_*` (+ 2 `browser_cdp_*`) decls in `Browser-Bridging-Header.h`; wired through
  `BrowserState.isVaultOpen`, a `RootView` host, an App-menu **"Passwords…"** item + `AppDelegate.showVault`,
  and a `BROWSER_VAULT_DEMO` auto-open harness. **Verified visually** in the running SwiftUI app: launched a
  clean build under an isolated `$HOME` with the demo vault seeded, screenshotted the surface rendering the
  seeded login (title + glass search + Lock + `Demo Login / octocat@example.com` row). The whole chain fired:
  core vault → `is_locked`/`list` FFI → Swift JSON decode → SwiftUI render. **Verification gotcha logged
  (memory):** `just build-macos` and `just run` build to *different* locations (default DerivedData vs
  `apps/macos/build/`), and two concurrent sessions clobber the shared DerivedData — build to a private
  `-derivedDataPath` and launch *that* binary. `strings`/`otool` don't surface this app's Swift literals, so
  don't gate on them — launch + screenshot.
- **🎉 Vault initialization + merge-import landed + verified 2026-07-07 (P2-13, user request).** The vault now
  has a real **setup flow** and import merges. **Init (log into an existing account, ADR-0016):** a native
  "Set Up Passwords" screen (server URL + email + master password + optional 2FA) → `browser_vault_setup` →
  `service::setup` (init + `unlock_online` + persist `vault-config.json`). Later launches auto-init from that
  config (`service::start()` in `embed::initialize`); the UI shows setup only when unconfigured, unlock when
  configured-but-locked, the list when unlocked. **Import merges:** `manager::import` now syncs, runs
  `import::filter_new` (dedupe by host+username, or name when URI-less; **add-only**, never overwrites), and
  uploads only the new — so re-importing is idempotent and a second source only adds what's new. UI: an
  **Import** button (toolbar + a prominent empty-state CTA) → `NSOpenPanel` (CSV / Bitwarden JSON) →
  `browser_vault_import_*` → "Imported N logins". FFI: `browser_vault_is_configured` + `browser_vault_setup`
  added. **Verified:** `just verify` green — 128 core tests (3 new merge-filter); **both screens screenshotted
  in the running app** (the setup form + the list with the Import button). The `setup`→`unlock_online` and the
  import upload are already live-validated against Vaultwarden; `filter_new` is unit-tested. **Remains for
  `later`:** add/edit login, an import preview/confirm step, in-app account **registration**, and settings
  (change server / lock timeout).
- **🎉 Add/edit/delete login + a sidebar entry point landed + verified 2026-07-07 (P2-13, user request).**
  The vault is now a real **manager**, not a viewer, and is reachable from the UI. **CRUD:** single-cipher
  `create_login` (`POST /ciphers`) / `update_login` (`PUT /ciphers/{id}`) / `delete_login` (`DELETE`) — added
  `PUT`/`DELETE` to the transport; `cipher_entry` (the encrypt step) is factored out of bulk import and
  shared. `manager::{add,update,delete}_login` (op → resync); `service` + FFI
  (`browser_vault_{add,update,delete}_login`, `browser_vault_login` for full detail). To round-trip an edit
  (a PUT replaces the cipher), the sync `Login` model now decrypts **notes** too, and `browser_vault_login`
  returns the full record (incl. password — an explicit gesture, ADR-0017). **UI:** an **Add** toolbar button
  + a `.sheet` `LoginEditor` (name/username/password/website/TOTP/notes), row **Edit…** (context menu +
  double-click, prefilled) and **Delete**. **UI access point:** a **key button in the sidebar's bottom bar**
  (`SpaceSwitcher`) → `showVault`, alongside the App-menu item. **Verified:** `just verify` green (128 core
  tests); **screenshotted** the prefilled Edit sheet (masked password proves the full-detail round-trip), the
  Add/Import/Lock toolbar, and the sidebar key icon. CRUD requests are the same live-validated encrypt+POST
  path; the save-against-a-live-server round-trip rides the same Vaultwarden validation. **P2-13 now covers**
  setup · unlock · list/search · reveal · merge-import · **add/edit/delete** · sidebar access. **Remains for
  `later`:** settings (change server / lock timeout), an import preview/confirm step, and in-app registration.
- **P2-1 is the structural spine for P2-2..P2-5.** The core `tabs` model gains a tab *type* (pinned / today /
  favorite); rename, clear-ephemeral, revert-on-close, and global favorites all hang off it. It realizes the
  mvp1 "pinned/today split" fast-follow, extended with the global-favorite class (P2-5).
- **P2-3 semantics:** base behavior clears all today tabs except the active one; with a multi-selection the
  selected tabs are the ones **kept** and "Clear" removes the rest of the ephemeral set.
- **P2-4 is Arc's exact pinned-tab behavior:** a pinned tab holds a saved URL; you may navigate it during a
  session, but "closing" it reverts it to the saved URL and keeps it in place (reopenable) rather than removing
  it. "Replace URL with current" rewrites the saved URL to wherever you've navigated.
- **P2-6 (downloads) is independent of the tab work** — no dep on P2-1. mvp1 scopes downloads UI out of MVP
  ("history/downloads UI beyond command-bar suggestions"), so it lands here in the complete-browser phase.

## Phase 3 — 1.0: "power-user & developer browser" · *not current; partially itemized*

Goal (rm §5.3): the differentiators that create the "I can't go back" effect for power users and developers.
**Only on-request captures are itemized here**; the full §5.3 scope (Commands in ⌘T + ⌘K palette, keyboard-first
nav, tab search/suspension, split view, named sessions, sync v1, docked DevTools, network inspection/throttling,
…) stays at phase granularity in `roadmap.md` until Phase 2 nears exit.

### Built-in developer/design tools (user request 2026-07-07)

A lightweight visual **inspect / measure / convert** layer *beside* the existing full DevTools (ADR-0021) and
CDP surface (P0-4) — the native-absorb answer to the DevTools-adjacent extensions (ColorZilla, page rulers,
font inspectors, VisBug). Two mechanisms, **both already built**: **native overlays / OS pickers** (pure-pixel
tools) and the **document-start injection substrate** (P2-14 / ADR-0017, DOM-aware tools). Home: a new
**Developer menu** + command-bar entries + a toggle-able inspect-overlay HUD — a sibling `NSView` over the CEF
view, **never** SwiftUI-restructuring the platform view (CLAUDE.md gotcha). All `later` (Phase 1 is current
focus); the user asked to plan, not start.

| ID | Item | Status | Est | Deps | Refs |
|----|------|--------|-----|------|------|
| P3-1 | Developer Tools **foundation** — a **Developer menu** + shared inspect-overlay HUD host (sibling `NSView` over the CEF view) + command-bar entries + an "inspect mode" toggle the individual tools ride. Built once (like the injection substrate); **may warrant an ADR** at build time (native-overlay vs. injected-script split; positioned beside full DevTools). | later | ~35k | P1-M2 | rm §5.3; user request 2026-07-07 |
| P3-2 | **Eyedropper** color picker — native `NSColorSampler` (samples any pixel, even off-page); copy hex/rgb/hsl/oklch; recent-swatch history; live contrast ratio between two samples. *Native OS picker.* | later | ~25k | P3-1 | rm §5.3; user request 2026-07-07 (v1) |
| P3-3 | **Measure / guides** ruler — drop draggable guides + **element-to-element distances** (alt-hover, Figma-style) with px readout. *Injected content script.* | later | ~35k | P3-1 · P2-14 | rm §5.3; user request 2026-07-07 (v1) |
| P3-4 | **Element peek** — hover any element → floating box-model (margin/padding/size) + font + colors, no panel; **copy-as-CSS**. *Injected content script.* | later | ~30k | P3-1 · P2-14 | rm §5.3; user request 2026-07-07 (v1) |
| P3-5 | **Font inspector** (WhatFont-class capability, **deliberately not named that**) — hover → the *rendered* `font-family`; click-to-pin → full type panel (family/style/weight/size/line-height/letter-spacing/color) + **source detection** (`@font-face` src / Google / Adobe Fonts / system). Overlaps P3-4; ship standalone or as peek's type-detail expansion — a UX call, same injected script. *Injected content script.* | later | ~30k | P3-1 · P2-14 | rm §5.3; user request 2026-07-07 (v1) |
| P3-6 | **Screenshot / capture suite** — region (native overlay), selected-element, and full-page (stitched via CEF/CDP); copy or save. The screenshot half of rm §5.3's "internal capture system"; **video/screen recording deferred** (own row when it nears). *Native region + CEF/CDP.* | later | ~40k | P3-1 | rm §5.3 (power-user #7); user request 2026-07-07 (v1) |
| P3-7 | **Palette + type extractor** — every color + font-family the page actually uses, click-to-copy. Pairs with P3-2/P3-5. *Injected content script.* | later | ~25k | P3-1 · P2-14 | rm §5.3; user request 2026-07-07 (v2) |
| P3-8 | **Encoder scratchpad** — base64 / JWT-decode / URL enc-dec / timestamp↔date / regex tester / JSON pretty-print in one command-bar-invoked panel; **auto-pretty-print raw-JSON responses**. Core pure-fn logic + native panel (no page injection except the JSON auto-format). *Core + native.* | later | ~30k | P3-1 | rm §5.3; user request 2026-07-07 (v2) |
| P3-9 | **Contrast / a11y overlay** — WCAG contrast flags on low-contrast text, heading (H1–H6) outline, missing-`alt` audit, tab-order. *Injected content script.* | later | ~30k | P3-1 · P2-14 | rm §5.3; user request 2026-07-07 (v2) |
| P3-10 | **Device / responsive frames** — resize viewport to device presets, rotate, DPR toggle (CDP `Emulation.*`). **Is** rm §5.3's "device/responsive emulation" dev-toolbox line — cross-ref, not a new capability. *CDP.* | later | ~30k | P3-1 · P0-4 | rm §5.3 (dev toolbox); user request 2026-07-07 (v2) |
| P3-11 | **QR for current URL** — core QR generator + native popover; hand the page off to a phone. Trivial, high delight. *Core + native.* | later | ~12k | P3-1 | rm §5.3; user request 2026-07-07 (v2) |
| P3-12 | **Tech-stack detector** (Wappalyzer-class) — fingerprint framework / CDN / analytics via an injected probe → a summary popover. *Injected content script.* | later | ~30k | P3-1 · P2-14 | rm §5.3; user request 2026-07-07 (v2) |
| P3-13 | **Split view** — side-by-side tab tiling (2-pane to start; grid/workspaces later). rm §5.3 Tier-3 differentiator; Firefox-baseline parity. Depends on stable multi-tab CEF hosting + focus routing. | later | ~90k | P1-M2 · P0-1 | rm §5.3 · user request 2026-07-08 |
| P3-14 | **Commands foundation** — core `commands` registry + built-in Commands (tab-by-URL switch, copy URL, …) + `CommandsProvider` in the **⌘T command bar** (unified ranking with URLs/tabs/history). ADR-0034 slice 1. | later | ~40k | P1-M2 · P1-M3 | ADR-0034 · rm §5.3; user request 2026-07-08 |
| P3-15 | **Commands — user scripts** — TypeScript runtime + `CommandContext` API (`tabs`, `spaces`, `navigate`, `process.exec`, `path`, `clipboard`, `notify`) + per-file store under `commands/` + settings **Commands** section (list, enable/disable, native editor, import/export) + safety gates (show-before-enable, process permission). ADR-0034 slice 2. | later | ~55k | P3-14 | ADR-0034 · rm §5.3 |
| P3-16 | **Commands — vibe-code UX** — generated API type stubs + editor autocomplete, inline test runner (current-context `:repl`), built-in cookbook import (Glide-class examples: calendar tab, GitHub clone, …). Manual authoring only; in-app agentic create/edit is P4-1. ADR-0034 slice 3. | later | ~35k | P3-15 | ADR-0034 · rm §5.3 |
| P3-17 | **Command palette (⌘K)** — action-first floating panel sharing the Command registry + built-in browser actions; **no** URL/search navigation (that stays ⌘T). ADR-0034 secondary surface. | later | ~30k | P3-14 | ADR-0034 · rm §5.3 |
| P3-18 | **Commands — keyboard bindings** — optional per-Command shortcuts; pairs with keyboard-first / hint mode. ADR-0034 slice 4. | later | ~25k | P3-15 · P3-17 | ADR-0034 · rm §5.3 |

Notes:
- **Sequencing.** **P3-1 is the spine** — the Developer menu + overlay HUD host every other row hangs off;
  build it first, then the tools are independent vertical slices (core/inject → FFI → Developer-menu item +
  shortcut → overlay behavior). **v1** = P3-2…P3-6 (the "inspect" toolkit the user named: *color, space, box,
  type, capture*); **v2** = P3-7…P3-12 (extractor, convert, a11y, responsive, QR, detect).
- **Mechanism split (the reason this is cheap).** Nothing here needs new engine plumbing. Pure-pixel tools
  (eyedropper, region capture) ride **native overlays / OS pickers**; DOM-aware tools (measure, peek, font,
  palette, a11y, tech-detect) ride the **P2-14 injection substrate** (ADR-0017) as additional consumers —
  the "one hard thing, many riders" payoff, same as cosmetic ad-block + consent. Responsive frames reuse the
  **CDP** surface (P0-4). So each tool is small; the substrate/menu/overlay shell is the only shared cost (P3-1).
- **Overlap notes (one source of truth).** P3-6 is the *screenshot* slice of rm §5.3's capture system (video
  recording stays a separate later row); P3-10 *is* the roadmap's "device/responsive emulation" line, pulled
  in here as a concrete row. P3-4 (element peek) and P3-5 (font inspector) overlap — decide standalone-vs-merged
  when P3-4/P3-5 are picked, not now.
- **Positioning.** These are the *lightweight* complement to full Chromium DevTools (separate window, ADR-0021)
  and the automation CDP surface (P0-4) — not a reimplementation. The pitch is native-absorb (rm §3.1): the
  DevTools-adjacent extensions, built in.
- **Commands (ADR-0034, user 2026-07-08).** Glide-class browser-chrome hackability — distinct from Tweaks
  (page injection) and the external CDP port. **⌘T stays the primary unified surface** (URLs + tabs + history +
  search + Commands); **⌘K palette is additive** (action-first, same registry). Sequencing: P3-14 (registry +
  built-ins + bar provider) → P3-15 (user TS + settings) → P3-16 (vibe-code UX) → P3-17 (⌘K palette) →
  P3-18 (keyboard bindings). P3-17 can parallel P3-15 once P3-14 lands. **Agentic in-app create/edit** of
  Commands (natural language → TS in the manager, same consent gates) is a **Phase 4 AI feature** (P4-1) —
  not required for Commands v1; pairs with the agent layer (spec §19).

## Phase 4 — Top-tier · *partially itemized*

Goal (rm §5.4): genuine top-tier, cross-platform, AI done cleanly. Most scope stays at phase granularity
in `roadmap.md`; pull more rows as Phase 3 nears exit.

| ID | Item | Status | Est | Deps | Refs |
|----|------|--------|-----|------|------|
| P4-1 | **Commands — agentic create/edit** — in-app AI authoring in the Commands manager: describe a workflow (or iterate in chat), agent generates/edits TypeScript via `command_upsert`, user reviews code + enables with existing consent gates; execution stays human-initiated. Chrome-automation counterpart to agent-authored Tweaks (§20). | later | ~40k | P3-15 · P3-16 · agent layer | ADR-0034 · spec §19/§21 · rm §5.4; user request 2026-07-08 |

## Decisions pending (unblock future work)

The rm §10 forks. Each carries a recommendation in the roadmap; **none blocks the current Phase 0–1 window**,
but each gates Phase 2+ scope. Resolving one is a valid "what's next?" outcome (Mode 2) — record the call as
an ADR and unblock dependents.

| Fork | Decision | Gates | Status |
|----|------|------|------|
| 3.1 | Extensions → native-absorb + userscripts | Phase 2–3 | open (recommended) |
| 3.2 | DRM / codecs posture | **Phase 2** — decide before scope locks | open (recommended) |
| 3.3 | CEF release lane (LTS + Stable hot-jump) | Phase 2 | open (recommended) |
| 3.4 | Sync as a 1.0 pillar (design early) | Phase 3 | open (recommended) |
| 3.5 | AI layer posture | Phase 3–4 | open (recommended) |
| 3.6 | Mobile post-1.0 | post-1.0 | open (recommended) |

---

*Seeded 2026-07-06 from roadmap §5 + mvp1.md; reconciled against the working tree the same day (first
"what's next?" run). Headline finding: a single-browser embed shell is well underway (nav, DevTools popup,
find, command bar, autocomplete, favicons, glass sidebar), but the multi-tab core spine (P1-M1) and focus-
routing proof (P0-1) — the two things everything else hangs on — are still open.*
