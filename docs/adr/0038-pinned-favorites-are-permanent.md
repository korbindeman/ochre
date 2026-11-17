# ADR-0038: Pinned favorites are permanent - close deselects, never dismisses

- **Status:** Accepted - close reset behavior refined by [ADR-0056](0056-pinned-url-separate-from-live-navigation.md)
- **Date:** 2026-07-09
- **Supersedes:** the dismiss semantics of [ADR-0032](0032-pinned-tabs-favorites.md)

## Context

ADR-0032 introduced Arc-style **favorites** - the spacious favicon icons below the URL bar, always
resident in memory - and modeled a user "close" as **dismiss**: a `dismissed: bool` flag hid the icon
from the strip (the browser stayed loaded), and dismissed favorites collected behind a three-dot
"closed pinned tabs" ellipsis menu that restored them.

In practice this fought the mental model favorites exist to serve. A favorite is meant to be permanent
furniture: the icon is *always* there, the tab is *always* resident. The dismiss model made the icon
disappear on ⌘W (and via a −-on-hover button on the active icon), which reads as "I lost my pin," not
"I closed the view." Arc's actual behavior is that the favorite icon never leaves the strip; a close
just drops you back to the canvas.

## Decision

**Favorites are permanent.** A user close only *deselects* - it never hides, moves, or tears down.

- **Remove the `dismissed` concept entirely** - the `Tab.dismissed` field, `SavedTab.dismissed`
  (persistence), the `TabView.dismissed` snapshot field, `dismiss_favorite`, `restore_favorite`, the
  `browser_tab_restore_favorite` FFI/bridge symbol, and every `!dismissed` filter.
- **`close_tab` on a favorite calls `deselect_favorite`** - if the favorite is its space's active tab,
  repair the active pointer to the canvas (`next_canvas_tab_after_close`, which only auto-selects
  *open* tabs, else empty); otherwise a no-op. The tab, its loaded browser, its `favorite` flag, and
  its icon all stay.
- **UI:** drop the −-on-hover close button and the ellipsis "closed pinned tabs" menu. The icon is
  click-to-focus; ⌘W deselects. Only **Unpin** (context menu / drag to the tree) and **Delete**
  (context menu) ever remove a favorite.
- The `MAX_VISIBLE_FAVORITES` (6) cap now counts all favorites (no visible/dismissed distinction);
  `favorite_count` replaces `visible_favorite_count`.

## Alternatives

- **Keep dismiss, just relabel it** - rejected: the whole complaint is that the *icon vanishes*. Any
  model where a favorite can leave the strip on close reproduces the bug.
- **⌘W discards the live page and reloads the pinned URL** (true Arc) - rejected for now: the user's
  model is "always loaded / technically it's always there." Keep-loaded/just-deselect is simpler and
  matches the request; revisit if a "clear favorite" affordance is later wanted (would be additive,
  not a return to dismiss).
- **Keep `dismissed` in the schema for forward-compat** - rejected: dead fields are dead code. Old
  `session.json` files that still carry `"dismissed": true` deserialize fine - serde ignores the
  unknown field - and the favorite simply loads as a normal permanent icon.

## Consequences

- Snapshot JSON and `session.json` lose the `dismissed` field on each tab row; the Swift `TabItem` /
  `TabDTO` and the C bridging header lose it too. No migration needed (serde drops unknowns).
- The FFI surface shrinks by one symbol (`browser_tab_restore_favorite`).
- Closing an active favorite is now indistinguishable from selecting the canvas - no strip mutation,
  so the strip never animates on ⌘W.
- Covered by the core unit test `favorite_close_deselects_but_keeps_icon` and the existing
  `just drag-selftest` favorites coverage (pin/unpin/reorder unchanged).
