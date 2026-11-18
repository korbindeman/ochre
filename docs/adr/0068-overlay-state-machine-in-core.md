# ADR-0068: The command bar and address edit are one core state machine

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

The command bar and the in-place address editor each wrote a global `Navigation.route` (new tab or current tab) when they opened and closed, and every suggestion row read it on commit. Close notifications arrive late: the command bar's panel resigns key only after the next surface is already up. So this sequence sent an address edit's Return to a new tab:

1. Open the command bar.
2. Click the address chip.
3. The command bar resigns key and resets the route.

That is one cause of "navigating doesn't work" (P1-51, user report 2026-11-18). P1-46 asks for explicit UI state machines, owned by the core, with Swift only rendering state.

## Decision

- **`crates/core/src/overlay.rs` holds the state machine.** The state is `Overlay::None | CommandBar { dest } | AddressEdit`, so at most one surface is up and the commit destination is part of the state. `step` is pure.
- **A close names the surface it closes.** `Close(Kind)` is ignored unless that surface is the one up, so a late close cannot reset a newer surface.
- **A commit with no surface up is dropped.** A commit from a surface navigates to where that surface points (the command bar's `dest`, or the current tab for an address edit) and returns to `None`.
- **The host sends intents and renders the result.** Swift calls `browser_overlay_send`. `AppDelegate.renderOverlay` is the only code that shows or hides these surfaces. Focus returns to the page only when the new state is `None`, so a surface that took over keeps the keyboard.
- **Removed:** `Navigation.route`. The command bar's own `toggle`, and its route writes on show and hide.

## Alternatives

- **Patch `Navigation.route` resets with ordering guards.** Rejected. The bug class is "a global written by several surfaces". Any new surface would reopen it.
- **A general statechart engine (SCXML subset, as in vitae).** Deferred. One enum and one pure `step` cover today's surfaces, and the tests walk every reachable state. Adopt the engine when vitae lands, or when the surface count makes the enum unwieldy.
- **Keep the machine in Swift.** Rejected. A second frontend would have to reimplement it (spec: core logic is frontend-agnostic), and Swift has no cheap exhaustive path test.

## Consequences

- **Tests:** they walk all reachable states. They check that opening shows exactly that surface, that a close affects only the surface it names, that a commit lands where the visible surface points, that a stale commit does nothing, and the reported sequence. `just newtab-selftest` replays the sequence in the app.
- **Not covered yet (P1-46 continues):** find, and the History, Passwords and Settings pages (`isHistoryOpen` and the like). These still use Swift booleans. Fold them in next when friction reports show them misbehaving.
