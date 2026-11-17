# ADR-0058: Native menu projects browser commands into Cmd-T

- **Status:** Accepted, refines the built-in slice of [ADR-0034](0034-commands-browser-automation.md)
- **Date:** 2026-08-14

## Context

ADR-0034 requires built-in browser actions in the unified Cmd-T surface. The
macOS app already has a canonical registry for those actions: its native menu.
Each menu item owns its title, shortcut, selector, target, and contextual
validation. Re-declaring built-ins in a second core registry would duplicate
availability rules and drift from the discoverable native menu.

Dynamic Space names are application state rather than static menu items, but
they must be searchable in the same surface.

## Decision

On macOS, Cmd-T projects every enabled AppDelegate-targeted native menu action
through `BrowserCommandsProvider`. It recursively reads the current menu,
invokes the same validation used by AppKit, fuzzy-matches title plus menu path,
and dispatches the live menu item. Adding a future browser menu action therefore
adds a command without another registration step.

Opening Cmd-T, closing the window, and quitting are excluded. The first is
recursive; the latter two are unsafe as fuzzy suggestions. Space switching uses
a separate dynamic provider that emits one command per current Space.

This is the platform adapter for built-in browser actions, not a replacement for
ADR-0034's user-defined Commands registry. User TypeScript Commands remain
core-owned so all frontends share storage, runtime, permissions, and query
semantics when P3-15 lands.

## Consequences

- Cmd-T can run arbitrary enabled browser actions without a hard-coded shortlist.
- Native menu validation and command-bar availability cannot diverge on macOS.
- The sidebar URL editor remains navigation-only; browser commands are scoped to
  Cmd-T.
- Windows will project its own canonical native command surface while sharing
  user-defined Commands through the later core registry.

