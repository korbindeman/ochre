# ADR-0031: Private windows (incognito)

- **Status:** Accepted
- **Date:** 2026-07-08

## Context

Phase 2 table-stakes includes private browsing. The product already ships
network ad-block, reject-first cookie consent (ADR-0026), and a Bitwarden-
compatible vault. Users expect a distinct private *window* — not a Space —
with honest privacy boundaries and the same annoyance-blocking posture.

## Decision

- **Private windows only** — a separate `NSWindow` with an ephemeral flat tab
  list. Not a Space; no save/pin/folders/session restore.
- **CEF isolation** — each private window gets a dedicated `RequestContext` with
  an empty `cache_path` (Chromium off-the-record / in-memory profile).
- **Core privacy gates** — no `history::record_visit`, no favicon persistence,
  no session writes for private tabs.
- **Adblock + cookie consent** — same global engines; per-site shield
  allowlists are **ephemeral per private window** (never persisted).
- **Passwords default off** — first private window shows a dismissable prompt;
  enabling is opt-in via `settings.private_vault_enabled`. Save-password is
  never offered in private.
- **Downloads** — warn on the private empty state; no delete-on-close (files
  on disk are the user's responsibility).

## Alternatives

- **Private as a Space** — rejected: Spaces persist and would leak URLs into
  `session.json` and command-bar history.
- **Shared incognito context across private windows** — rejected: separate
  windows should not share cookies.
- **Delete downloads on close** — rejected as low value vs. complexity.

## Consequences

- Multi-window shell: focused-window routing (`browser_set_focused_window`),
  per-window `BrowserState`, extended event callback `(type, window_id, tab_id,
  payload)`.
- Quit only when the last window's browsers are gone; closing the normal window
  while private windows remain keeps the app alive.