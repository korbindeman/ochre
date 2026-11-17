# ADR-0041: Pinned↔floating sidebar swap cross-fades in place, not slide-past-slide

- **Status:** Accepted
- **Date:** 2026-07-09

## Context

The sidebar exists as **two AppKit-hosted SwiftUI islands** in `RootView` (ADR-0019's
"beside content" model): `sidebarHost` - the pinned column, flush in the chrome slot at
x=0 - and `floatingHost` - the hover/empty-canvas card (`FloatingSidebar`), inset 8pt with
rounded corners + a drop shadow. Both render the same `Sidebar` content at the same width.

Each hides by **sliding off-screen left** (the column parks at `x = chrome - chromeWidth`,
the card at `x = -sidebarWidth - 16`). That is correct when a sidebar appears from or
collapses to *nothing* (pin-from-collapsed, hover reveal, collapse-with-a-tab). But when the
floating card is already on screen and the user pins (⌘S) - or unpins back onto an empty
canvas, where the card immediately floats - **two independent animations fire at once**:
`setSidebarPinned` slides the column in from the left while `syncFloatingSidebar` slides the
near-identical card out to the left. The user sees two copies of the same panel swipe past
each other at the window edge instead of the sidebar simply settling into place.

## Decision

For a **direct pinned↔floating swap only** - pinning while the card is revealed, or unpinning
onto an empty canvas - neither host translates horizontally. A one-shot `inPlaceSwap` flag
(set in `setSidebarPinned`, read by `applyLayout`) **freezes the column at x=0 and the card at
its inset**, and the two **cross-dissolve** (column alpha ↔ card alpha, matched at 0.16s) while
the content region reflows to make/reclaim room. Every other transition keeps its slide:
`inPlace` is gated on `floatingShown` (pinning) / `state.isFloatingSidebarShown` (unpinning),
so collapse-to-nothing, reveal-from-nothing, and pin-from-collapsed are unaffected. Under
Reduce Motion the swap snaps (no cross-fade), matching the rest of the sidebar.

## Alternatives

- **Unify the two hosts into one sidebar view whose "pinned-ness" is a style parameter**
  (inset / corner radius / shadow / height) - rejected for now. It is the cleaner long-term
  shape (every transition becomes one view animating in place, killing this whole class of
  cross-slide artifacts), but it is a large change touching URL-editor ownership
  (`Sidebar.isVisible` per `SidebarVariant`), the resize-handle geometry, z-order, and the
  CEF-resize-during-transition path - disproportionate risk against the "never restructure
  around the CEF view" constraint for a polish fix. **Do not "simplify" the two hosts into one
  without reopening this** - the split is deliberate.
- **Geometrically morph the card into the column** (animate inset→0, corners→square,
  shadow→none) - rejected. The card's rounding/shadow live *inside* the SwiftUI island and
  will not animate across the AppKit frame animation, so the morph can't be driven cleanly
  from `RootView`; it would need the unified host above.
- **Leave the double-slide** - rejected; it is the reported defect.

## Consequences

- The swap reads as the sidebar settling in place: a soft in-place cross-fade, **not** a
  geometric morph of the card's corners/shadow. If that dissolve ever looks too soft/ghosted
  (two identical panels offset by the 8pt inset blend during the fade), that is the signal to
  escalate to the unified-host approach above - this ADR would be superseded, not edited.
- `RootView` now manages `sidebarHost.alphaValue` / `isHidden`, which it previously never
  touched (the column was always opaque, hidden only by going off-screen). `setSidebarPinned`
  restores alpha=1 / unhides on any non-in-place transition so an interrupted cross-fade can't
  strand the column mid-alpha or hidden.
- Scope is minimal and contained to `RootView.setSidebarPinned` / `applyLayout`; no core, FFI,
  or `BrowserState` changes (it keys off the existing derived `isFloatingSidebarShown`).
