# ADR-0022: App-UI surfaces and tabs are mutually exclusive at the UI layer, not the core

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

App-UI surfaces (History, Passwords — [ADR-0010](0010-native-app-ui-surfaces.md)) render in
the content area as `NSHostingView`s raised over the active tab's CEF view, gated by two Swift
booleans (`isHistoryOpen` / `isVaultOpen`). They are **not** tabs: the core tab model never
learns about them.

That split left a seam. The core's active tab is unchanged while a surface covers it, so:

- The sidebar kept drawing its active-tab highlight over a tab the user couldn't see or reach —
  it *looked* selected on a screen it had no relationship to.
- Clicking a sidebar tab called `browser_tab_select` directly, which switched the core's active
  tab and swapped the CEF view *underneath* the still-raised surface — a selection that did
  nothing visible. The screen read as "bugged."
- Nothing dismissed a surface but Escape or opening a tab; there was no visible close control.
- The two surfaces could stack (open one over the other), and `⌘W` only recognized History.

The real question this forced: when a surface opens, should the core's **active tab** be
cleared (a genuine deselect), or should exclusivity live entirely in the UI layer while the core
keeps its active tab?

## Decision

Exclusivity is enforced **at the UI layer**; the core keeps its active tab throughout.

- Opening a surface **suppresses the sidebar's active-tab highlight** (`isActive` is gated on
  `!isSurfaceOpen`) — visually no tab is selected while a surface covers the canvas.
- Opening one surface **closes the other** (`openHistory` / `openVault` set the sibling false),
  so only one ever covers the content area.
- Clicking a sidebar tab routes through `BrowserState.selectTab`, which **dismisses any open
  surface, then selects the tab and returns keyboard focus to its web view**. (The core already
  refocuses when the *visible* tab changes; the explicit `browser_focus_active_tab()` covers
  re-selecting the tab that was active behind the surface, where the visible tab doesn't change.)
- Each surface has a **leading ✕ close button**; `⌘W` dismisses whichever surface is open before
  it would touch the tab/window beneath. Dismissal returns to the underlying active tab — the
  "dismissed back to the active tab" model ADR-0010 already stated.

## Alternatives

- **Core-level deselect (new `browser_tab_deselect` FFI + a "tabs exist, none active" state)** —
  rejected. It collides with the empty-state logic, which reads `activeTabId == nil` as "no tabs"
  (`reconcileEmptyStateReveal` would float the sidebar out as if empty when a surface closed);
  it raises undefined session-persistence questions (what's the active tab on relaunch?); and it
  makes closing a surface dump the user onto a blank canvas instead of the page they were on.
  Large, riskier, and worse UX for no benefit the UI-level approach doesn't already deliver.
- **Leave selection as-is, only add a close button** — rejected: doesn't fix the core complaint
  (a tab reads as selected on a screen you can't navigate from; clicking a tab does nothing
  visible).
- **Trailing ✕ after the Clear/Import/Lock controls** — rejected: crowds the content-manipulation
  controls and invites a mis-click that dismisses the surface. Leading, clear of them, reads as
  "leave this screen."

## Consequences

- Dismissing a surface (✕ / Escape / ⌘W / opening or selecting a tab) always returns to the tab
  that was active — consistent with ADR-0010 and with overlay conventions (Escape puts you back).
- No core/FFI change; the core tab model stays oblivious to surfaces, honoring "native layers are
  view + binding only."
- The **URL-bar chip still shows the active tab's address** while a surface is open (it reflects
  the core's active tab, which is unchanged by design). This is a known, deliberate limitation of
  the UI-level approach — the *highlight* is the tab-selection signal we clear; the address chip
  is a separate surface. If it later reads as inconsistent, neutralizing the chip while a surface
  is open is a self-contained follow-up, not a reopening of this decision.
- The ✕ button is duplicated inline across the two surface files (as `SearchField`/`HostFavicon`
  already are); a third surface is the trigger to extract a shared `SurfaceCloseButton`.
