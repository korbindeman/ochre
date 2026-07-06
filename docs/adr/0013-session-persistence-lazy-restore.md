# ADR-0013: Session persistence — write-through JSON + lazy tab hydration, core tab ids decoupled from CEF

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

The tab/space/folder tree (ADR-0006) lived only in memory: quitting the app lost every tab. The
user's requirement is explicit — **all tabs stay unless they are closed**, including ephemeral
("today") tabs, and history must persist (history already does, in SQLite — `crates/core/src/history.rs`).
Session restore is also a roadmap Tier-0 capability (rm §4).

Three forces make this a real decision:

1. **Identity.** `TabId` *was* CEF's browser identifier, which restarts at 1 every session — a
   persisted tab id could not survive a relaunch, and a restored tab must exist in the model before
   any CEF browser exists for it.
2. **Cost of restore.** Eagerly recreating a CEF browser per saved tab spawns a renderer process per
   tab at launch — directly against the lightness thesis (rm §1.2, §8.2).
3. **Durability.** A "save on quit" step is fragile (crashes, force-quits, the CEF teardown path);
   and the app-quit teardown closes every browser through the *same* handlers as a user tab-close,
   so a naive implementation persists an emptying store and wipes the session on every quit.

## Decision

- **Core-assigned `TabId`s** from a persisted counter; a tab's CEF browser is `Option<Browser>`.
  `None` = *suspended* (restored, not yet hydrated). Engine handlers still speak CEF identifiers;
  the `tabs::note_*` functions translate (a popup/DevTools browser translates to no tab).
- **Lazy hydration.** At launch the whole restored tree appears in the sidebar immediately; a tab's
  browser is created the first time the tab is shown (`browser_tab_restore` FFI → `embed::restore_tab`
  → `tabs::attach_browser`). Tabs never revisited never cost a renderer.
- **Write-through JSON** (`session.json` next to `history.sqlite`): every structural change, plus
  URL/title updates, rewrites the file via atomic temp+rename (`crates/core/src/session.rs`). The
  file is always current; kill -9 at any moment loses nothing. Loading sanitizes (dangling refs
  dropped, folder⇒pinned re-imposed, active pointers repaired), so a corrupt file degrades to a
  fresh start, never a panic.
- **Shutdown mask.** `request_close_all` (the single app-quit funnel) first calls
  `tabs::begin_shutdown()`; persistence is then frozen so the quit-time browser drain is not
  recorded as the user closing tabs.
- **All tabs persist**, ephemeral included — per the user's call. ADR-0006's pinned/ephemeral split
  still governs *placement* (sections, folders, space→folder transform closing ephemerals); it no
  longer implies ephemeral tabs are session-scoped. (Arc-style auto-archival of today tabs would be
  a separate, later decision.)

## Alternatives

- **Keep `TabId` = CEF identifier, remap on restore** — rejected: the host keys views and events by
  tab id; ids changing at hydration (or colliding with a new session's CEF ids) pushes remapping
  complexity into every consumer. Decoupling fixes identity at the root, and the `Option<Browser>`
  state is exactly what M5 tab suspension needs anyway.
- **Eager restore (recreate every browser at launch)** — rejected: a renderer process per saved tab
  at startup; slow launch, high memory — the anti-thesis. Mainstream browsers lazy-restore for the
  same reason.
- **SQLite storage (extend `history.sqlite`)** — rejected: the session is one small tree, read once
  and rewritten whole; a single JSON document is simpler, human-inspectable, and atomic via rename.
  History stays SQLite (row-oriented, unbounded growth, incremental writes).
- **Save on quit only** — rejected: loses the session on crash/force-quit and depends on ordering
  inside CEF teardown; write-through makes durability a non-event.
- **Persist only pinned tabs (ADR-0006's original leaning)** — rejected by the user's explicit
  requirement: all tabs stay unless closed.

## Consequences

- Tab identity is stable across sessions; CEF identifiers are an engine-side detail translated at
  the `note_*` boundary. `current_url` in `client.rs` is keyed by CEF id (popups have pages too).
- A suspended active tab means the single-target verbs (`navigate`, `go_back`, `current_url`, …)
  no-op until hydration — in practice the host hydrates the active tab in the same runloop pass
  that shows it.
- The session file is rewritten on every title/URL change during browsing. Writes are a few KB on
  the main thread (same thread the history SQLite writes already ride); coalescing is a later
  optimization if it ever shows up in profiles.
- The restore logic (`TabStore::from_saved`, sanitization, the shutdown mask) is unit-tested
  without CEF because suspended tabs need no browser objects.
- Sidebar UX for suspended tabs (dimming, à la Arc) is possible later by exposing hydration state
  in the snapshot; deliberately not added until a design wants it.
