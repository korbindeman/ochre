# ADR-0069: Pages and the find bar join the core surface machine

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

ADR-0068 moved the command bar and the address edit into a pure core machine. It left out the find bar and the History, Passwords and Settings pages. Those still used Swift booleans with the cross-surface rules spread across call sites: opening a page ended the address edit by hand, `isSurfaceOpen` guarded address editing, and nothing closed the find bar when a page covered the web view.

## Decision

- **The machine's state is now `Ui { overlay, find, pages }`.** `overlay` is app-wide (at most one input surface). `find` is the one window whose find bar is up. `pages` maps each window to the page covering its web view; absent means the web view.
- **The cross-surface rules live in `step`:**
  - Opening a page closes the command bar or address edit, and that window's find bar.
  - Find and address editing are ignored while a page covers that window.
  - `ShowWeb` returns a window to its tab.
  - A closed window forgets its page and find bar.
- **Window ids:** 0 is the normal window, 1 and up are private windows, and a little window is `-(tab id + 1)`.
- **The host renders only.** `BrowserState.isHistoryOpen` and the other page flags are now `private(set)` and set only by the renderer, so nothing else can write them. The find bar shows and hides only from the renderer. Focus returns to the web view only when no input surface is up and no page covers the key window.
- **The FFI is now `browser_surface_send(input, arg, window, url)`**, replacing `browser_overlay_send`.

## Alternatives

- **Separate machines for pages and find.** Rejected. The bugs are in the interactions between surfaces. Separate machines would put those rules back in host glue.
- **Per-window overlay state.** Rejected. Only the key window takes keyboard input, and the command bar is one app-wide panel. A per-window overlay would allow two input surfaces at once.

## Consequences

- **Tests:** the reachable-state tests now walk 111 states over two windows. Two invariants are asserted: find never sits under a page, and an address edit needs a window showing its web view. `just newtab-selftest` checks the find and page rules in the app.
- **Sidebar tab activation and drag stay where they are.** Activation is already core-owned (`TabStore`), and drag has its own resolver (ADR-0035). P1-46 is closed. New surfaces join this machine.
