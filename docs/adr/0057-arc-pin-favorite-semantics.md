# ADR-0057: Arc Pin and Favorite semantics

- **Status:** Accepted, refines [ADR-0032](0032-pinned-tabs-favorites.md), [ADR-0038](0038-pinned-favorites-are-permanent.md), and [ADR-0056](0056-pinned-url-separate-from-live-navigation.md)
- **Date:** 2026-08-14

## Context

The data model already had two durable tab kinds, but the interface named them
backwards relative to Arc. The per-Space persistent row was called Saved, while
the global icon-strip item was called Pinned. As a result, Pin Tab created a
global Favorite and the per-Space action was Save Tab.

Closing was also inconsistent. An active pinned row used a non-destructive minus
action, while the same row became a destructive delete action when inactive.
Activity should not change whether a close destroys durable organization.

## Decision

The user-facing taxonomy is:

- **Favorite:** a global item in the icon strip, available in every Space.
- **Pinned:** a persistent tab in one Space, loose or inside a folder.
- **Open:** a non-pinned working-set tab in one Space.

Pin Tab and Cmd-D create a per-Space pin. Unpin Tab returns it to Open. Move to
Favorites moves a tab to the global strip. Remove from Favorites returns it to
Open. The existing core fields (`pinned` and `favorite`) already represent these
meanings and do not need a migration.

Closing any pinned tab is non-destructive, active or inactive. It returns the
live URL to the durable pinned URL, suspends any browser, and keeps the row in
the Pinned section. Delete Pinned Tab is the explicit destructive action. A
Favorite keeps ADR-0038's permanent icon behavior; Delete Favorite is explicit.

Every Space has a collapsible Pinned section. Expansion is core-owned,
snapshot-carried, and persisted with the Space. Existing sessions default to
expanded. A collapsed section peeks its active pinned tab so current context is
never invisible. Its header remains a drop target while collapsed.

## Consequences

- Pin, Favorite, and Open now match Arc's visible concepts.
- Hover close and context-menu close have the same durable behavior.
- Deletion is always explicit for Pinned tabs and Favorites.
- Space persistence grows one backward-compatible `pinned_expanded` field.
- Internal helpers or serialized fields may retain older names where changing
  them would create compatibility churn, but user-facing copy follows this ADR.

