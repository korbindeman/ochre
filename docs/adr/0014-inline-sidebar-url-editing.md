# ADR-0014: Inline sidebar URL editing - the chip edits in place, not via the command bar

- **Status:** Accepted
- **Date:** 2026-07-06
- **Supersedes:** [ADR-0004](0004-sidebar-url-chip.md)

## Context

ADR-0004 added the sidebar URL chip as a pure *display*: it showed the active
tab's address, and clicking it (or ⌘L) opened the **command bar** in
edit-in-place mode. URL *entry* happened only in the transient center overlay.
That ADR explicitly rejected an editable inline field - citing a cramped ~200px
column and a reluctance to add a second entry surface - and closed with: "if a
future change makes it an editable field … supersede this ADR rather than
quietly widen the chip."

This is that change. Routing a URL edit to a center-screen overlay is a detour:
the address you want to change is right there in the sidebar, and Arc - the
interaction model we copy (style §1) - edits it *in place*, expanding the
sidebar field with its results below it. Bouncing focus to the middle of the
window to change the URL of the tab you're looking at reads as indirection, not
speed. The concerns that killed the inline field in ADR-0004 turned out to be
smaller than assumed: the autocomplete engine (`SuggestionController` /
`AutocompleteList`) was already decoupled and reusable, and in-window focus
stealing from CEF is already proven by the History surface.

## Decision

The sidebar URL chip **edits inline**. Clicking it - or ⌘L - turns the chip into
a live search/URL field *in the sidebar*, with its autocomplete dropping down as
a floating card over the tab list. Commit (Return / choosing a row) loads in the
active tab; Escape or clicking away restores the chip and returns focus to the
page.

- The **command bar stays the ⌘T new-tab surface** - it no longer has an
  edit-in-place mode. The two entry surfaces are now cleanly split: center
  overlay = *new tab*; inline sidebar field = *change this tab's address*.
- The inline field reuses the command bar's machinery: the shared
  `AutocompleteField` (the text field + inline-completion behaviour, extracted
  from the command bar) and `AutocompleteList`, driven by a `URLBarModel` that
  owns the editing flag and a `SuggestionController`.
- Only the **visible** sidebar instance hosts the live field (pinned column, or
  the hover-revealed floating copy), so an off-screen instance can't focus a
  hidden field and swallow keystrokes. ⌘L reveals a collapsed sidebar first; the
  reveal auto-dismiss is suspended while editing.

## Alternatives

- **Keep ADR-0004 (route the click into the command bar)** - rejected: it's the
  status quo this ADR reverses. The center overlay for a same-tab address edit is
  indirection; Arc (our model) and every mainstream browser edit the location
  where it's shown.
- **A panel anchored to the chip** (reuse the command bar's `NSPanel`, floated at
  the chip instead of centered) - rejected: it satisfies "looks inline" while
  still being a separate key window with the panel's focus/compositing dance. The
  pinned sidebar sits *beside* CEF (no GPU-composite conflict) and in-window
  focus stealing is already proven (History), so a real embedded field is
  simpler and reads as part of the sidebar.
- **Everything inline, retire the command bar** (⌘T also opens an inline field) -
  rejected for now: a brand-new tab has no sidebar address to edit in place, and
  the center command bar is the signature ⌘T interaction (style §6, spec §9).
  Splitting new-tab (center) from edit-current (inline) matches Arc exactly.

## Consequences

- The command bar loses its `Mode` / edit-in-place path; `CommandBarController`
  is new-tab-only. The field logic it shared with the inline bar now lives in the
  reusable `AutocompleteField`.
- `Navigation.route` is set to `.currentTab` for the duration of an inline edit
  and reset to `.newTab` on dismissal - the same per-session routing the command
  bar's edit mode used, now driven by `URLBarModel`.
- The sidebar is no longer a zero-entry-surface chrome. This is a deliberate
  reversal of ADR-0004's "one entry path" stance: there are now two, but they are
  disjoint by purpose (new tab vs. this tab), not redundant.
- `BrowserState` / `RootView` gain a small coupling to `URLBarModel.isEditing`
  (end editing on pin toggle; don't auto-collapse a revealed sidebar mid-edit) so
  the floating-sidebar case can't strand a focused field.
- Style §3, §6, and §13 are updated to describe inline entry for the current
  tab and reserve the "no persistent full-width bar" rule for the actual
  full-width toolbar it was written against.
