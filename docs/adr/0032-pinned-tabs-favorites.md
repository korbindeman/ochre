# ADR-0032: Pinned tabs (favorites) — Arc-style resident icons, distinct from saved

- **Status:** Accepted
- **Date:** 2026-07-08

## Context

ADR-0006 introduced a **saved** section (`pinned: bool` on tabs — folders + loose saved) and an
**open** (ephemeral) section. Style §4 and backlog P2-36 call for a third class: **Arc-style pinned
tabs** — spacious favicon icons below the URL bar, always resident in memory, where a user "close" only
dismisses the icon (the browser stays loaded). This is *not* the same as saving a tab into the tree
below New Tab.

The core already overloads `pinned` for "saved." Renaming it would churn session files, FFI, and every
call site. A new field is required.

## Decision

**Core model (`crates/core/src/tabs.rs`):**

- Add `favorite: bool` — the tab is an Arc-style pinned icon in the active space's strip below the URL
  bar. Mutually exclusive with the saved section: `favorite ⇒ !pinned && folder.is_none()`.
- Add `dismissed: bool` — the favorite is hidden from the strip after a user close. The tab and its
  browser **remain** in the store; only the icon is gone. Restoring clears `dismissed`.
- **Close semantics:** `close_tab` on a favorite sets `dismissed = true` (and repairs the active
  pointer) — it does **not** remove the tab, record it on the ⌘⇧T stack, or tear down the browser.
  Non-favorites keep the existing close path.
- **Always loaded:** favorite tabs are pre-hydrated by the host on structure change / space switch
  (all favorites in the active space, including dismissed). Visibility still follows the single active
  tab (`set_visible`).
- **Persistence:** `favorite` and `dismissed` are written through to `session.json` with the rest of
  the tree.
- **Relocation:** extend the drag-and-drop primitive with a `favorite` destination section (reorder
  within the strip, pin from open/saved, unpin back to open).

**UI (macOS):** a horizontal favicon strip between the URL bar and the scrollable tree; context menu
**Pin Tab** / **Unpin Tab**; drag-and-drop within the strip and from rows into it. User-facing copy is
"Pin" even though the core field is `favorite`.

**Deferred (P2-4):** saved-URL revert on close and "Replace URL with current." **Deferred (P2-5):**
global favorites across all spaces.

## Alternatives

- **Reuse `pinned` for both saved and icon-strip tabs** — rejected: the sections are visually and
  behaviorally different (list rows vs. icon strip; dismiss-vs-destroy on close). Overloading one flag
  would fight the saved/folder model from ADR-0006.
- **Destroy the browser on favorite dismiss and lazy-restore on re-show** — rejected: contradicts
  "always loaded" and Arc's instant re-open from the strip.
- **Separate `FavoriteTab` entity outside the tab list** — rejected: duplicates identity, CEF mapping,
  and event wiring; favorites are still tabs, just a different section.

## Consequences

- Snapshot JSON gains `favorite` and `dismissed` on each tab row; older session files deserialize with
  both `false` (serde default).
- `browser_tab_move` gains a leading `favorite` parameter — the only FFI consumer is the macOS app.
- The host must preload favorite-tab browsers (not only the active tab) and must not detach CEF views
  on favorite dismiss.
- Parity ledger and style §4 get a row when the slice lands end-to-end.