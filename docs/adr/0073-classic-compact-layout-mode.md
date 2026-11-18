# ADR-0073: Classic layout mode - Safari-compact tab row, Arc behavior kept

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

Ochre's layout is Arc's: a vertical-tab sidebar holding the controls, the URL chip
(ADR-0014), the tab tree and the Space switcher (style §1-§3). A Settings choice
already moves the address bar from the sidebar into the page (`AddressBarPlacement`,
P1-28). Korbin wants a second, more classical layout that Chrome and Safari users
recognize on sight, while keeping Spaces, favorites, pinned tabs and folders.

Style §3 and §13 forbid a persistent full-width toolbar. A classic mode has to
reverse that for one layout, so the choice needs a record. The open questions were
what "classic" looks like, where Spaces go, how the sidebar's tab kinds fit a
horizontal row, whether behavior changes with the layout, and how the setting is
shaped.

## Decision

1. **Classic = Safari compact.** One row under the traffic lights holds
   back/forward, the Space picker, the tabs, and nothing else. The **active tab is
   the address field**: it shows the URL, edits inline on click or ⌘L (the same
   edit-in-place as the sidebar chip, ADR-0014), and carries reload, the adblock
   shield and the vault key inside its pill.
2. **Spaces: a picker in the row** (Safari Tab Groups idiom). ⌃1-9 and the
   trackpad swipe still switch Spaces. An optional sidebar listing only Spaces is a
   later toggle, not part of the first slice.
3. **Favorites and pinned tabs become compact icon-only tabs** at the left of the
   row (Chrome pinned-tab idiom). A folder becomes a menu button that lists its
   tabs. The tab model does not change.
4. **Behavior stays Arc's in both layouts.** ⌘T opens the centered command bar,
   a Space with no tabs shows the empty page, ⌘W on the empty page closes the
   window (ADR-0037). Only the layout differs.
5. **No auto-archive, in any layout.** Ochre has never auto-archived Open tabs;
   they persist across launches (ADR-0013). If an auto-archive ever lands it is
   opt-in, off by default.
6. **Overflow:** inactive tabs shrink to a minimum width, then the row scrolls
   sideways. The active tab keeps a width that stays usable for editing the URL.
7. **One setting: Layout = Sidebar | Classic**, global, stored in the core
   settings. The existing "Address bar: Sidebar | Page" choice becomes a sub-option
   of the Sidebar layout. Classic always has its address field in the active tab.

## Alternatives

- **Safari "separate" toolbar (full-width URL field, tab row below)** - rejected:
  two rows of chrome. Compact is closer to the existing chip model (the URL lives in
  a tab-shaped surface) and keeps the content area tallest.
- **Chrome layout (tabs in the title bar, toolbar below)** - rejected for the same
  two-row cost, and it is the least native look on macOS.
- **Classic behavior with the classic layout** (⌘T opens a new tab page, closing the
  last tab closes the window) - rejected: Korbin wants one behavior model. The
  centered command bar already reads as "new tab, type a URL", so Chrome muscle
  memory still works.
- **A third value on the existing setting (Sidebar | Page | Classic)** - rejected:
  it mixes where the URL goes with the whole layout and invites incoherent
  combinations.
- **Per-window layout** - rejected: no use case, and it doubles the state the
  window chrome has to track.
- **Keep the full Arc sidebar, hidden, in Classic** - rejected: two places to find
  tabs defeats "familiar on sight".

## Consequences

- Style §3/§13 gain a Classic exception: the compact tab row is the one persistent
  horizontal chrome allowed, and only in the Classic layout.
- The compact row is a new AppKit-owned surface; SwiftUI renders only inside hosted
  islands, never around the CEF view (CLAUDE.md gotcha). Tab drag inside the row
  needs its own resolver, guarded like `drag-selftest`.
- `focus-selftest` must cover ⌘L into the active-tab field and Escape back to the
  page in Classic.
- Width pressure is the main risk: many pinned icon tabs plus the Space picker can
  crowd the row. Overflow (decision 6) is the answer; folder menus keep it bounded.
- The layout setting lives in the core, so a future frontend (ADR-0064) reads the
  same value. Every Classic row element maps to an existing core operation; no
  tab-model change.
