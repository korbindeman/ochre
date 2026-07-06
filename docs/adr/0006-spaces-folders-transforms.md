# ADR-0006: Spaces, folders, pinned/ephemeral tabs, and the folder↔space transform

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

M4 is spaces. The user also asked for **tab folders** and a **bidirectional folder↔space transform**.
`mvp1.md` deferred Arc's pinned-vs-ephemeral split ("simplified to one flat list for MVP; the pinned/
today split is a fast-follow"). But the transform can't be *correct* without that split: a folder is a
group of **persistent** tabs, so promoting a folder to a space — and demoting a space to a folder — only
makes sense for the tabs that are meant to persist. Ephemeral "today" tabs are transient and have no
place in a folder. So the split is a prerequisite, pulled forward here.

## Decision

**The model (in the Rust core, `crates/core/src/tabs.rs`):**

- A **tab** gains `pinned: bool` and `folder: Option<FolderId>`, with the invariant *`folder.is_some()`
  ⇒ `pinned`* (only persistent tabs live in folders). **New tabs are ephemeral** (Arc's "today").
- A **folder** `{id, space, name}` lives in a space's pinned section and holds pinned tabs.
- A **space** `{id, name, active: Option<TabId>}` is a named container rendered as a **pinned section**
  (folders + loose pinned tabs) above a **today section** (ephemeral tabs). It remembers its own active
  tab, so switching spaces restores where you were.
- Pinning moves a tab to the pinned section; unpinning returns it to today and clears its folder. Moving
  a tab into a folder auto-pins it. Deleting a folder **keeps** its tabs (they drop to loose pinned) — no
  silent tab loss. Deleting a space **closes** its tabs; the last space can't be deleted.

**The transforms:**

- **Folder → Space**: create a new space named after the folder; its (all-pinned) tabs become the new
  space's pinned tabs; remove the folder; switch to the new space.
- **Space → Folder**: the source space's **pinned** tabs become a new folder in the **active** space; the
  source space's **ephemeral tabs are closed** (only persistent survives — the crux of "only the
  persistent ones stay"); the emptied source space is deleted. Offered only for a *non-active* space, so
  the target (the active space) is unambiguous and no picker is needed.

## Alternatives

- **Keep the flat list (the mvp simplification)** — rejected: the transform is meaningless without a
  persistent/ephemeral distinction; you'd be promoting throwaway tabs into a durable space.
- **Ephemeral tabs survive the space→folder demotion** — rejected: they're transient by definition; Arc
  drops them, and carrying them into a folder contradicts what a folder *is*.
- **Space→folder with a destination picker** — deferred: constraining it to non-active spaces makes the
  target implicit (the active space) and avoids a modal. Revisit if drag-to-a-specific-space is wanted.
- **Interleaved pinned/today ordering** — deferred: render folders then loose pinned then today, in
  insertion order (drag-reorder is a later pass, consistent with tabs).

## Consequences

- The sidebar gains two sections per space plus a space switcher; reverses the mvp "one flat list" note
  (mvp updated to point here).
- Per-space active-tab memory falls out of the `Space.active` field.
- Structure now changes in bursts (a transform reparents/closes many tabs) — handled by the snapshot
  re-pull of **ADR-0005** rather than fine-grained events.
- Deferred for a follow-up: drag reorder / drag-between-folders, space color/theme (still out per mvp),
  and cross-session persistence of pinned tabs & spaces (the model is persist-ready; storage is later).
