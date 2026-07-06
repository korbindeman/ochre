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
| P0-1 | CEF ↔ native keyboard / focus routing | wip | ~60k | — | spec §15#1 · rm §5.0 · mvp M2 |
| P0-2 | Signed + hardened-runtime bundle → passkey / Touch-ID login | blocked | ~50k | Apple Developer enrollment + entitlement request (your action) | rm §5.0 / §6.1 |
| P0-3 | Native ad-block on the CEF request path | next | ~30k | — | rm §5.0 · spec §11 · mvp M5 |
| P0-4 | CDP surface (loopback, consent-gated) | next | ~25k | — | rm §5.0 / §3.5 |

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
  fmt). **Re-test pending** to confirm the two `NSWindow` cases now become clean page-focus; then P0-1 → done.
- **P0-4:** DevTools is wired as a toggle (`browser_toggle_dev_tools`, ⌘⌥I + F12) plus a page context menu
  with **Inspect Element** (opens DevTools at the clicked node) — the "opens DevTools" Phase-0 exit criterion
  is met. What remains for P0-4 is the loopback, consent-gated **CDP automation** surface.
- **Also already built** (single-browser, uncommitted): nav + back/fwd/reload/zoom, find-in-page, history
  frecency autocomplete + favicons, and the glass-sidebar shell scaffolding.

## Phase 1 — MVP1 (= mvp1.md) · *current focus*

Goal (rm §5.1 / mvp): the Arc interaction model in native chrome, good enough for **you** to daily-drive.
**Exit:** the mvp "Definition of done".

| ID | Item | Status | Est | Deps | Refs |
|----|------|--------|-----|------|------|
| P1-M1 | Core tab model + multi-tab CEF (Rust core + FFI event stream) | done | ~90k | — | mvp M1 · ADR-0003 |
| P1-6 | Empty state (zero-tab) UI + launch-to-empty | done | ~25k | P1-M1 | mvp (§new-tab); user request 2026-07-06 |
| P1-M2 | SwiftUI shell + vertical-tab glass sidebar | blocked | ~110k | P1-M1 · P0-1 | mvp M2 |
| P1-M3 | Cmd+T command bar (Liquid Glass morph) | blocked | ~80k | P1-M2 | mvp M3 |
| P1-M4 | Spaces switcher + multi-space UX | done | ~60k | P1-M1 | mvp M4 · ADR-0005/0006 |
| P1-7 | Tab folders + pinned/ephemeral split + folder↔space transform | done | ~110k | P1-M4 | ADR-0006; user request 2026-07-06 |
| P1-M5 | Adblock integration + polish (shortcuts, tab suspension, animation) | blocked | ~70k | P1-M2 (P0-3 covers the request-path core) | mvp M5 |
| P1-8 | Split `client.rs` (727 L) → `client/{handlers,favicon,devtools}.rs` — one file mixes 8+ CEF handlers, favicon capture, DevTools, error-page HTML | next | ~20k | — | code §3 |
| P1-9 | Split Swift view monoliths — `BrowserRootView.swift` (8+ view types) + `CommandBar.swift` (panel + key-handling + sizing) into per-component files | next | ~25k | — | code §3 |
| P1-10 | Omnibox-grade command-bar autocomplete — core fuzzy ranking + match highlighting + inline autocomplete | done | ~55k | — | ADR-0011 · style §6 · spec §9; user request 2026-07-06 |
| P1-11 | Sidebar layout: Arc control placement — nav controls share the traffic-light row, New Tab row heads the today section | done | ~15k | P1-6 | style §3; user request 2026-07-06 |
| P1-12 | Space emoji icons (core model + switcher UI) + default 🏠 Home space visible at launch (initial snapshot pull) | done | ~30k | P1-M4 | style §5; user request 2026-07-06 |
| P1-13 | History page polish — macOS 26 toolbar idiom (large title + Liquid Glass search capsule + glass Clear, `safeAreaBar` scroll-under), spacious rows + hover highlight | done | ~15k | — | style §15; user request 2026-07-06 |
| P1-14 | Traffic lights rework — custom-drawn in the sidebar, native buttons hidden; fixes drift/stale visibility from AppKit button relayout (worst on the beta OS) after three real-button approaches failed | done | ~90k | P1-11 | ADR-0012 · style §3; user bug report 2026-07-06 |
| P1-15 | Session persistence — tabs/spaces/folders survive restart (**all** tabs stay unless closed), lazy per-tab hydration | done | ~90k | P1-M4 | ADR-0013; user request 2026-07-06 |
| P1-16 | Persist sidebar pinned state across launches (UserDefaults, beside sidebar width) | done | ~10k | — | style §3; user request 2026-07-06 |
| P1-17 | Reopen closed tab (⌘⇧T) — faithful restore into original space/folder/pinned/slot, capped in-session stack | done | ~25k | P1-15 | style §4; user request 2026-07-06 |

Notes:
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
- **P1-8 / P1-9 (2026-07-06) — file-monolith splits surfaced by `code-style.md` §3.** No function in the tree
  is a tangled monolith; the pressure is purely at the *file* level, so these are decomposition-only (move
  code into concern-scoped files, adjust visibility/imports, no behavior change). **Sequencing matters:**
  **P1-8** (`client.rs`) is independent — safe to pick anytime. **P1-9** touches the two Swift files that
  **P1-M2 / P1-M3** will heavily expand; both milestones are `blocked` on P0-1, so the files aren't churning
  now and a split is safe — but best done *just before* M2/M3 open (or folded into them) so the decomposition
  benefits that work rather than being redone. Not a hard dep; a sequencing note.
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

## Phase 2 — Public Beta: "a complete browser" · *not current; partially itemized*

Goal (rm §5.2): a stranger switches without a week-one wall — the Minimum Viable Daily Driver (rm §4.1) — and
you can ship security patches. **Only on-request captures are itemized here**; the full §5.2 scope (import,
session restore, settings UI, vault, release infra, …) stays at phase granularity in `roadmap.md` until
Phase 1 nears exit.

| ID | Item | Status | Est | Deps | Refs |
|----|------|--------|-----|------|------|
| P2-1 | Tab taxonomy in core — a tab *type*: pinned (per-space, persistent) vs. today (ephemeral) | later | ~40k | P1-M1 | mvp §Arc-UX ("pinned/today split is a fast-follow"); user 2026-07-06 |
| P2-2 | Rename tab — double-click for inline rename + context-menu "Rename" | later | ~20k | P2-1 | user 2026-07-06 |
| P2-3 | Clear ephemeral (today) tabs — keep the active one; multi-select marks tabs to keep, "Clear" removes the rest | later | ~25k | P2-1 | user 2026-07-06 |
| P2-4 | Pinned-tab saved-URL semantics — navigate freely, "close" reverts to the saved URL (tab persists, reopenable), + "Replace URL with current" | later | ~30k | P2-1 | Arc pinned-tab behavior; user 2026-07-06 |
| P2-5 | Global pinned tabs (Favorites) — pinned across *all* spaces, distinct from the per-space persistent tabs | later | ~35k | P2-1 | user 2026-07-06 |
| P2-6 | Downloads indicator + section (Tier-1 downloads manager) | later | ~40k | — | rm §4.1 / §5.2; user 2026-07-06 |
| P2-7 | Traffic-light native-parity polish — option-key alternate glyphs, VoiceOver labels/traits, green zoom-vs-fullscreen nuance (deferred from P1-14's custom-drawn lights) | later | ~20k | P1-14 | ADR-0012 |

Notes:
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

## Phases 3–4 — not yet itemized

1.0 (rm §5.3) and Top-tier (rm §5.4) live in `roadmap.md` at phase granularity. Pull items into this backlog
as Phase 2 nears.

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
