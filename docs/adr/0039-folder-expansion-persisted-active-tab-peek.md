# ADR-0039: Folder expansion lives in the core tree; a collapsed folder peeks the active tab

- **Status:** Accepted
- **Date:** 2026-07-09

## Context

Sidebar folders (ADR-0006) are collapsible, but expand/collapse was view-local
SwiftUI `@State` on `FolderRow`, defaulting to collapsed. Two problems fell out
of that:

1. **It wasn't remembered.** The state reset not only on relaunch but on every
   space switch — switching away tore down the folder views and their `@State`,
   so returning showed every folder collapsed again.
2. **A collapsed folder hid the active tab entirely.** A collapsed folder simply
   didn't render its children, with no special-casing for the active tab. If the
   current tab lived in a collapsed folder it appeared *nowhere* in the sidebar —
   and because the selection pill is sourced from the active row
   (`matchedGeometryEffect`), there was no active-tab affordance at all while its
   page was on screen. Arc never hides the current tab.

Expansion state is part of the tree every frontend renders, and folder ids are
core-assigned — so a host-local store (UserDefaults keyed by folder id) would be
a second, frontend-specific persistence mechanism the next frontend would have
to reimplement, and would orphan keys as folders come and go.

## Decision

**Expansion is a field on the core `Folder` model**, persisted in `session.json`
(`SavedFolder.expanded`) and carried in the structure snapshot
(`FolderView.expanded`), exactly like `name` and `parent`. Toggling a folder
calls `browser_set_folder_expanded(id, expanded)` → `tabs::set_folder_expanded`,
a `structural` change that writes through and re-pulls the snapshot. `FolderRow`
holds no local collapse state; it renders `folder.expanded`.

**Defaults by creation context** (the field is set explicitly at each site):

- User-created folders open **expanded** — `create_folder`,
  `create_folder_with_tabs`, and `space_to_folder`: a folder you just made, or
  the tabs you just grouped, should be visible, not hidden behind a header.
- Bulk Arc-import folders stay **collapsed** — a large import shouldn't unroll
  into a wall of tabs; expand what you need.
- A pre-ADR-0039 session (no `expanded` field) restores **collapsed** via
  `#[serde(default)]`, matching how folders always looked at launch before.

**A collapsed folder peeks the active tab.** When collapsed, `FolderRow` renders
a single display-only row for the active tab if it lives in the folder or any
nested descendant (`activeDescendantTab(of:)` walks the tab's folder-parent
chain), and tints the folder glyph. Walking *up* from the tab means the
*nearest* collapsed ancestor peeks it, so it works at any nesting depth. The
peek is transient — it reads `activeTabId`, never the persisted `expanded` flag,
so landing on a tab in a collapsed folder never silently rewrites the user's
collapse preference.

## Alternatives

- **Keep expansion host-side, persist in UserDefaults.** Rejected: a second,
  per-frontend persistence path keyed by core-assigned ids; Windows/Linux would
  reimplement it, and it orphans keys as folders are deleted. Expansion is part
  of the shared tree, so it belongs where the tree lives — unlike genuinely
  view-only prefs (sidebar pinned/width) which stay host-side.
- **Auto-expand the whole folder when it holds the active tab.** Rejected (the
  reveal-style fork): it either mutates the persisted `expanded` flag — silently
  destroying the user's collapse intent — or needs a separate transient-expand
  override, and either way it unrolls every sibling tab when you only wanted to
  see the one current tab. Peeking the single active tab is more surgical and
  matches Arc.
- **Treat expansion as non-structural (a lighter event than `StructureChanged`).**
  Rejected: not worth a new event type. A toggle is a rare user action; reusing
  the `structural` write-through + snapshot re-pull keeps one code path and one
  source of truth. Visibility reconciliation is a no-op (the visible tab doesn't
  change), so the only added cost is a session write and a snapshot rebuild.

## Consequences

- Folder open/close survives relaunch and space switches, and animates with the
  same ambient spring as every other structural change (the old local
  `easeOut` toggle is gone).
- The active tab is always reachable in the sidebar; the tinted glyph + peeked
  row signal *where* it lives without expanding the folder.
- Each folder-construction site now states its `expanded` default explicitly —
  new call sites must pick one (there is no struct `Default`).
- The peek row is intentionally not a drag source and reports no drop frame, so a
  collapsed folder stays a single unit for drag-and-drop (a drop on the header
  files into it, per ADR-0035); reordering within a folder needs it expanded.
- Guarded by unit tests (`folder_expanded_survives_save_restore`,
  `folder_expanded_defaults_collapsed_when_absent`) and the standing
  `just drag-selftest` / `just close-selftest` regressions.
