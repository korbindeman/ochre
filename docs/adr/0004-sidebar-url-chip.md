# ADR-0004: Sidebar URL chip — a persistent current-tab address display

- **Status:** Superseded by [ADR-0014](0014-inline-sidebar-url-editing.md)
- **Date:** 2026-07-06

> **Superseded (2026-07-06):** the chip is no longer a display-only launcher into
> the command bar — it now edits **inline** in the sidebar. See
> [ADR-0014](0014-inline-sidebar-url-editing.md). The context and alternatives
> below remain the record of why the display-only chip was chosen first.

## Context

The style guide originally read "no persistent address bar — URL entry is the
transient command bar" in three places (§3, §6 anti-pattern in §13). That rule
was written to kill the *full-width* Chrome/Safari toolbar and keep the chrome
slim and content-first — a good instinct we still hold.

But it over-reached. Arc — the interaction model we copy (§1) — *does* show the
current tab's URL, as a compact chip at the top of the **sidebar**, not a
full-width bar. That chip is a genuine part of the model: it tells you where you
are, and clicking it is the mouse path to edit the address (the `Cmd+L`
edit-in-place §6 already specifies). Omitting it left the sidebar with no
location indicator at all — the user could not see the active tab's URL without
opening DevTools, which is a real gap, not a slim-chrome win.

So the question: does "no persistent address bar" forbid Arc's sidebar URL chip,
or only the full-width toolbar it was written against?

## Decision

Add a **persistent current-tab URL chip to the sidebar**, below the controls
row (Arc placement). It is a *display*, not an entry field:

- It shows the active tab's URL (scheme stripped), or a "Search or enter URL"
  placeholder when there is no active tab.
- Clicking it opens the **command bar** in edit-in-place mode — prefilled with
  the current URL (selected), committing in the active tab. With no active tab
  it opens the command bar for a new tab.
- URL *entry* still happens only in the transient command bar. The chip adds no
  new text-entry surface; it is a location display plus a launcher.

This keeps the guide's *intent* (slim chrome, entry via the command bar) while
correcting its overstatement. Style-guide §6 is updated to permit the sidebar
chip explicitly and reserve the prohibition for the full-width bar; §3 and §13
already say "full-width," which the chip is not.

## Alternatives

- **No URL bar at all (guide as originally written)** — rejected: leaves the
  sidebar with no way to see the active tab's location, which diverges from Arc
  (the model we copy) and from every mainstream browser. "Slim chrome" does not
  require *zero* location feedback.
- **A full-width persistent address bar** (Chrome/Safari toolbar) — rejected,
  unchanged: it eats vertical content space and is the exact Arc-killing
  anti-pattern §3/§13 were written for. The chip lives in the existing sidebar
  width and costs no content area.
- **An editable inline field in the sidebar** (type directly into the chip) —
  rejected: it would duplicate the command bar's autocomplete/keyboard flow in a
  cramped 200px column and reintroduce a persistent entry surface. Routing the
  click into the command bar reuses one entry path (§6) and keeps the chip a
  pure display.

## Consequences

- The command bar gains an **edit-in-place mode** (`Mode.editCurrent`): it
  prefills from `BrowserState.activeTab.url`, selects the text, and routes the
  commit to the active tab via `browser_navigate` instead of opening a new tab.
  A per-presentation `Navigation.route` carries the target so the generic
  autocomplete component stays mode-agnostic.
- `Cmd+L` ("Open Location…") is now wired (File menu), completing the §9 keyboard
  path that was previously listed but unimplemented — the chip is its mouse twin.
- The chip must stay a *display*: if a future change makes it an editable field,
  that reintroduces the persistent-entry surface this ADR deliberately avoided —
  supersede this ADR rather than quietly widen the chip.
- Spaces (§5, not yet built) sit between the chip and the tab list in Arc; the
  chip's placement leaves room for the space switcher above the tabs later.
