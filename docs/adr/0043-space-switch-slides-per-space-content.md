# ADR-0043: A space switch slides the sidebar as a filmstrip

- **Status:** Accepted
- **Date:** 2026-07-13

## Context

Switching spaces (a `SpacePill` click, ⌘⌥← / ⌘⌥→, or a two-finger swipe) all funnel
through `browser_select_space` / `browser_space_select_adjacent` into the core, which
emits a new structure snapshot. `BrowserState.applyStructure` swaps `activeSpaceId`, and
the sidebar's per-space tree - the space header, saved folders, loose saved, open tabs -
rebuilds for the new space.

Until now that rebuild rode only the ambient structural spring: rows cross-faded /
reflowed in place, so a *space* switch was visually indistinguishable from an ordinary tab
change within the current space. There was no directional cue - nothing conveyed that you
had moved *between* spaces, or which way. Arc (our interaction model) slides the whole
space column horizontally, which is what makes a space switch legible.

## Decision

Render the spaces' trees as a **horizontal filmstrip** and animate the strip's offset to
the active space. Every space is laid out side by side, one full-width panel each, inside
a clipped viewport; the strip is `offset(x: -activeIndex · width)`. A switch animates that
offset, so the outgoing and incoming panels **always travel together, in the switcher's
left→right direction**, by construction.

- **Why not SwiftUI insertion/removal transitions.** The first two attempts used
  `.id(activeSpaceId)` + a `.push` / asymmetric-`.move` transition. Both failed, and the
  failures are the whole reason for this design:
  1. Transition applied to `SpaceContents` *inside* the vertical `ScrollView` → the scroll
     view relayouts/clips away the horizontal offset, collapsing the slide to a pure
     opacity **cross-fade**.
  2. Transition lifted outside the scroll view (ZStack + `.clipped()`) → it slid, but
     **inconsistently**: SwiftUI captures the *outgoing* view's removal transition from an
     earlier render, so on a direction reversal the old panel left the way it had *entered*
     and the two panels slid *into each other*. There is no reliable per-render direction
     for a removal transition.
  The filmstrip sidesteps both: it never inserts/removes panels on a switch - all panels
  stay mounted and only a single `offset` animates, which is deterministic and
  interruption-safe (a rapid re-switch just re-targets the same offset).

- **Direction is implicit and spatial.** It falls out of the index in `state.spaces` (the
  same left→right order the switcher shows). Forward (higher index) → more-negative offset
  → strip slides left → the new panel, which sits to the right, comes in from the right.
  Non-adjacent jumps (clicking a distant pill) slide the whole distance, correctly. No
  direction hint is computed or plumbed anywhere.

- **Only the active panel is live.** `SpaceContents` takes a `spaceId` and an `interactive`
  flag; the inert panels render their rows (so the slide shows real content) but wire **no**
  drag, drop-frame reporting, taps, selection, or active-tab highlight. This is required,
  not cosmetic: the drop resolver (ADR-0035) is **Y-based**, so a second panel reporting
  frames at the same Y would corrupt drop targeting. `reportSidebarDropFrame(_:active:)` and
  `sidebarTabDrag(_:drag:enabled:)` gained a defaulted gate so the inert panels skip them
  and every existing caller is unchanged. `childFolders(of:)` is now space-agnostic (a
  folder's parent already determines its space) so non-active panels render their subfolders.

- **Rides the structural spring.** The offset animates on `applyStructure`'s existing
  animation transaction - the same `.spring(response: 0.24, dampingFraction: 0.88)` that
  springs every sidebar structure change. That path already applies the change *bare*
  (no `withAnimation`) on first population and under Reduce Motion, so the offset **snaps**
  in exactly those cases and slides only on a genuine switch - for free, with no view-side
  gate. A dedicated `.animation(value: activeSpaceId)` was tried and removed: it also fires
  on the launch hydration (active space `1 → restored`), sliding the initial space into
  place on every cold start. If the slide ever wants a more deliberate spring than the
  shared structural one, override it with a `.transaction` that retimes only when
  `txn.animation != nil` (so first-populate/Reduce-Motion still snap) - not a separate
  `.animation(value:)`.

## Scope - what slides and what stays put

Only the per-space tree filmstrip slides. Everything else is global or fixed chrome and
holds still, so the moving panels read against a stable frame:

- **Pinned favorites strip** - favorites are **permanent and global** (ADR-0038;
  `favoriteTabs` has no space filter), so they are not per-space content and don't slide.
- **Space switcher pills** - the navigation control you're operating; stays anchored.
- **Top nav row / URL bar** - fixed chrome.

Private windows have no spaces, so they render a single static panel (no filmstrip).

## Alternatives

- **`.id`-swap + `.transition` (cross-fade, then inconsistent slide)** - rejected; the two
  failure modes above are exactly what this ADR exists to avoid. A content slide must not
  depend on SwiftUI capturing a correct removal direction for a torn-out identity.
- **Transient two-panel overlay** (mount only the outgoing + incoming during a switch and
  animate their offsets, collapsing after) - rejected. It keeps steady state at a single
  panel, but needs a transition state machine with completion handling and interruption
  logic; the always-mounted filmstrip is simpler and interruption-safe for free. Space
  counts are small (a handful) and `SpaceContents` is a lightweight row list, so the cost
  of mounting them all is negligible.
- **Compute a direction hint in the core / `applyStructure` / `onChange`** - moot. The
  filmstrip needs no direction value at all; direction is implicit in the index. (An earlier
  revision of this ADR threaded a `spaceSlideDirection`; it was removed with the transition
  approach.)
- **Slide the whole sidebar column** (favorites strip + switcher included) - rejected. They
  aren't per-space; sliding them would imply they change with the space and would fight the
  pinned↔floating in-place swap (ADR-0041). The panels should move against fixed chrome.

## Consequences

- `SpaceContents` / `FolderRow` / `NewTabRow` now take `spaceId` + `interactive`;
  `BrowserState` gained `rootFolders(inSpace:)` / `looseSavedTabs(inSpace:)` /
  `openTabs(inSpace:)` (the bare `active…` forms delegate to `inSpace: activeSpaceId`). No
  core, FFI, or CEF-view involvement - contained to the SwiftUI sidebar island.
- All spaces' trees are mounted at once. Favicons for every space's tabs load eagerly as a
  result; acceptable at real space counts. If a profile ever carried enough spaces/tabs to
  make this heavy, the fallback is the transient two-panel overlay above - this ADR would be
  superseded, not edited.
- The drop resolver still sees exactly one live list, so sidebar drag (`just drag-selftest`,
  ADR-0035) is unaffected - the inert panels report no frames.
- Each space keeps its own scroll position (its panel has its own `ScrollView`), which is
  the desired Arc behavior.
