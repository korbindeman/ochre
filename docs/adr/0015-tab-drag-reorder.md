# ADR-0015: Tab drag-and-drop — implicit order, one relocation primitive, commit-on-drop

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

The sidebar needed drag-and-drop: reorder tabs inline, pin a "today" tab by dragging it into the
pinned section, and file a tab into (or out of) a folder. Two design questions had real forks.

**Where does order live?** The core owns tab structure (ADR-0006) as a flat `Vec<Tab>`; the sidebar's
sections (loose-pinned / today / per-folder) are `filter`s over it, so a tab's *position within a
section* is just its relative position in that Vec. Drag introduces a first-class "put this tab
*here*" operation, which forces the question of whether order should become explicit.

**How does the drag feel, given a core-owned model?** The model isn't view-local: every structural
change emits `StructureChanged` and writes through to `session.json` (ADR-0013). A drag that reorders
the live model on each hover-step would fan out FFI calls, snapshot re-pulls, and disk writes for a
gesture that may be cancelled.

## Decision

**Keep order implicit in `Vec<Tab>` order.** Add one primitive — `tabs::move_tab(id, pinned, folder,
before)` (core `TabStore::relocate_tab`) — that both re-parents a tab (pin/unpin, file into/out of a
folder, enforcing folder ⇒ pinned) and repositions it in the Vec: immediately before an anchor tab,
or at the destination section's end when there's no anchor. It subsumes pin and move-to-folder *with
a position*; the context-menu verbs (`set_pinned`, `move_to_folder`) stay for position-free actions.

**Track a single drop *slot* locally and commit exactly one `move_tab` on drop.** SwiftUI drop
delegates resolve the pointer to one of `{ before(tab), endOf(section), ontoFolder(folder) }` and
drive a live insertion indicator; the model is never mutated mid-drag. The New Tab row doubles as the
pin/unpin boundary — its top half pins (loose), its bottom half drops into "today" — so pinning works
even when the pinned section has no rows to aim at. The resulting snapshot re-render is spring-animated
in `BrowserState.reloadStructure` (which also gives create/close/select their motion).

## Alternatives

- **Explicit `order` field per tab (fractional or integer index)** — rejected. It adds a field to the
  model, the snapshot, and `session.json` (a migration), plus ordering arithmetic and rebalancing,
  to encode information the Vec already carries. No feature needs a stable order key independent of
  store position.
- **Live model reordering during the drag** (mutate on each `dropEntered`, the classic reorderable-
  `List` feel) — rejected. Against a core-owned, write-through model it means an FFI round-trip +
  full snapshot rebuild + `session.json` write per hover-step, all thrown away if the drag is
  cancelled. Commit-on-drop is one write, trivially cancellable, and easier to reason about.
- **Separate always-present pin/unpin drop zones** — rejected as clutter. Reusing the ever-present
  New Tab row as the boundary covers the empty-section cases without adding resting-state UI.
- **Convert the sidebar to a `List` with `.onMove`** — rejected. `.onMove` doesn't span the
  loose/today/folder sections, and a `List` would fight the custom row styling and section layout.

## Consequences

- The drag surface is thin: core logic is one pure `relocate_tab` method (unit-tested on plain
  `TabStore` values — reorder, pin-into-folder, unpin, foreign-folder rejection), behind a
  four-argument FFI call with `-1` sentinels for "no folder" / "section end".
- Order remains a property of store position, so anything that reorders `tabs` (restore, future
  features) must preserve intended section order — there's no separate key to lean on.
- Cross-*space* drag is intentionally out of scope (no space-switcher target yet); `move_tab` keeps a
  tab in its own space. Adding it later means repairing per-space active pointers on a space change —
  noted, not built.
- Verification: the core primitive and full stack are covered by unit tests + a green build; the drag
  *gesture* itself rides the standing interactive pass (P0-1 / M2), like the rest of the sidebar UI.
