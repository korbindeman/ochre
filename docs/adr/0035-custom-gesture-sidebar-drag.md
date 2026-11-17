# ADR-0035: Sidebar drag-and-drop - custom gesture-driven layer, one resolver, no system drag session

- **Status:** Accepted
- **Date:** 2026-07-09

## Context

ADR-0015 landed sidebar drag-and-drop on SwiftUI's system drag machinery: `onDrag`
(`NSItemProvider`) as the source and a mosaic of per-row / per-header / boundary
`onDrop` delegates as targets. In daily use it was unreliable enough to block
dogfooding (P1-27, user bug report 2026-07-08). The failure modes were structural,
not tunable:

1. **Layout-mutating insertion placeholder.** The ghost row was *inserted into the
   list* whenever the drop slot changed, shifting every row below it while the
   pointer was stationary - which changed the row under the pointer, which changed
   the slot, which moved the placeholder. A feedback loop producing a flickering
   indicator and off-by-one drops.
2. **Patchwork drop surface with dead zones.** Only views carrying an `onDrop`
   accepted the drop. Section gaps, headers, and the placeholder itself (hit-test
   disabled, nothing behind it) were dead: release there and `performDrop` never
   fired - the drag silently died *while the indicator was showing a target*.
3. **Drag-end by polling.** macOS `onDrag` has no end/cancel callback, so a 50 ms
   timer watched `NSEvent.pressedMouseButtons`, racing `performDrop` with a
   one-runloop grace period. When the timer won, the commit no-oped.

Beyond our own bugs, macOS `onDrag`/`onDrop` is a known-bad fit for this surface
(2025/26 survey): corrupted/missing drag previews, `onDrag` eating row clicks, no
source-side session observability, no autoscroll outside `List`, and drags started
in one subtree (pinned strip) not reliably reaching drop targets in another - we
already carried a special full-tree overlay just for favorite drags. No third-party
library covers cross-section + folder + strip semantics on macOS 15 (the one
maintained macOS-capable package, `visfitness/reorderable`, is single-container
only). Apple's real fix - `reorderable()` / `reorderContainer(for:in:)` - is
macOS 27, two majors above our floor. Production Arc-style sidebars (including,
by all evidence, Arc itself) use a custom gesture-driven drag, not the system
session.

## Decision

**Replace the system drag session with a custom gesture-driven layer.** The
ADR-0015 *model* is unchanged and still governs: order stays implicit in the
core's `Vec<Tab>`, a drag tracks one local `DropSlot`, and release commits exactly
one `browser_tab_move`. What changes is everything between the mouse and that
commit:

- **One coordinate space, one resolver, zero drop targets.** The sidebar root
  declares a named coordinate space spanning the pinned strip and the tab tree.
  Rows, strip icons, folder headers, the Open header, and the New Tab row report their frames into
  it (preference key). A single pure resolver maps pointer → slot over those
  frames - strip band resolved horizontally, list resolved vertically, nearest
  frame as fallback - so *every* point in the sidebar resolves to some slot.
  No `onDrop`, no delegates, no dead zones.
- **Rows feed a `DragGesture` (min distance 5 pt) into the shared drag model.**
  Explicit `begin → move → end` with a guaranteed `onEnded`: the commit is
  synchronous on release, the mouse-up poller race is gone. Release outside the
  sidebar, or Esc (local key monitor while dragging), cancels. A slow watchdog
  remains only as a leak guard for the no-`onEnded` pathology, not as the
  end-of-drag mechanism.
- **All drag visuals are a non-hit-testable overlay; mid-drag layout never
  changes.** The insertion indicator is a line positioned from reported frames
  (horizontal between rows, vertical between strip icons) and the drag preview is
  a ghost following the pointer - both rendered in an overlay on the sidebar
  root, `allowsHitTesting(false)` (CRASH-0001: overlays must stay out of the
  hit-test tree). The inserted placeholder rows are gone, killing the feedback
  loop.
- **High-frequency state is isolated.** Pointer position and reported frames live
  in a separate observable (`TabDragGeometry`) observed only by the overlay, so
  60–120 Hz pointer updates don't re-render every row. Rows observe only
  `draggingId`/`slot`.
- **Edge autoscroll** via the enclosing `NSScrollView` (grabbed by a probe view
  inside the scroll content): pointer in the viewport's edge zones scrolls the
  list and re-resolves the slot as frames update - impossible under `onDrop`.
- **Standing regression: `just drag-selftest`** (ADR-0018 pattern). Drives the
  pipeline at the gesture entry points (`dragChanged`/`dragEnded`) with pointer
  positions derived from the live reported frames - covering frame reporting,
  slot resolution (reorder, save via the Open boundary, favorite, folder, no-op suppression, cancel)
  and the single `move_tab` commit against the real core. It does not exercise
  AppKit's delivery of `DragGesture` events (Apple's code); that rides the
  interactive pass.

ADR-0015's gesture layer (drop delegates, placeholder rows, mouse-up poller) is
superseded; its model decisions stand.

## Alternatives

- **Patch the delegate mosaic** (unify on the favorite-drag overlay for all drags,
  overlay indicator, commit in the overlay's `performDrop`) - rejected. Cheaper
  (~30k tokens vs ~50–60k), and it would fix the feedback loop and dead zones,
  but it keeps `onDrag`: broken previews, rows that eat clicks, and drag-end by
  polling are framework behavior we can't fix from outside.
- **Third-party library** - rejected; none fits. The macOS-15 SwiftUI ecosystem
  has exactly one maintained candidate (`visfitness/reorderable`, single-container
  reorder only); the outline-shaped one (`Sameesunkaria/OutlineView`) is dead
  since 2023. Cross-section/folder/strip semantics would all remain hand-built
  around the dependency.
- **AppKit `NSOutlineView`** - rejected (again; see ADR-0015 on `List`). Correct
  drag semantics for free, but it replaces the entire SwiftUI sidebar rendering
  and fights the custom row chrome; also carries the macOS 15 `NSOutlineView`
  crash wave.
- **Keep the system drag session for its cross-window/out-of-window future** -
  rejected as a reason to stay. SwiftUI can't observe its own drag leaving the
  window pre-macOS 26 anyway; tear-off-to-new-window needs custom pointer
  tracking in either design, and the gesture layer is the better base for it.

## Consequences

- Core and FFI are untouched: `tabs::move_tab` / `browser_tab_move` remain the
  single relocation primitive; this is a Swift-only replacement of the event
  plumbing.
- Sidebar drags no longer participate in the OS drag session: no drag to other
  apps/windows from tab rows (was never supported), and future tear-off will be
  built on this gesture layer, not `NSDraggingSession`.
- The resolver + frame reporting are the one source of truth for drop geometry;
  any new sidebar drop target (e.g. the space switcher, deferred in ADR-0015)
  means a new frame kind + resolver case, not a new delegate.
- When the macOS floor reaches 27, `reorderable()`/`reorderContainer(for:in:)`
  is the exit path from all of this custom code; the slot/commit model maps onto
  it directly.
- Re-run `just drag-selftest` after sidebar layout changes and macOS SDK bumps,
  alongside `just focus-selftest`.
