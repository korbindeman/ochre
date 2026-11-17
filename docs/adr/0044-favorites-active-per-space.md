# ADR-0044: A favorite's active state is per-space, decoupled from its home space

- **Status:** Accepted
- **Date:** 2026-07-13

## Context

Favorites are the Arc-style global icon strip (ADR-0032): one resident tab shown
in *every* space, below the URL bar. But a favorite is still a `Tab`, and a `Tab`
carries a `space` - its *home* space (for an Arc import, the first imported space;
otherwise the space it was created in). Nothing in the sidebar layout uses that
field for favorites (their section ignores `space`), yet `select_tab` did:

```rust
store.active_space = tab.space;      // for a favorite, its home space
```

So clicking a favorite from any space snapped `active_space` to the favorite's
home space. Reported symptom: **"pinned tabs bring me back to the first space"** -
because imported favorites are all homed in the first imported space.

The underlying question this forced: *what does it mean for a global tab to be
"active"?* A favorite appears in every space, but the store models "the tab on the
canvas" as a single per-space pointer (`Space.active`, surfaced by `visible_tab`).

## Decision

**Selecting a favorite shows it on the current space's canvas without leaving the
space.** A favorite's active state is per-space: it can be `Space.active` for any
space - including several at once - independent of its home `space` field. Only a
non-favorite tab switches `active_space` to the space it belongs to.

The home `space` field is retained purely for storage/ownership (persistence,
reopen slotting, `same_section`); it no longer drives which space a favorite is
active in. Consequences that follow from the decoupling:

- **Deselect** (⌘W on a favorite, `deselect_favorite`) acts on `active_space`, not
  the home space - you drop it from the space you're viewing.
- **Removal** (`remove_tab`) repairs *every* space whose `active` named the tab,
  not just its home space, so no space is left pointing at a deleted favorite.
- **Restore** (session reload, space switching) accepts a favorite as any space's
  remembered active tab (`can_be_active_in`: `t.space == space || t.favorite`),
  so a favorite on a non-home canvas survives relaunch and space switches.

## Alternatives

- **Un-home favorites entirely (drop the `space` field).** Rejected: reopen (⌘⇧T)
  slots a closed favorite back into its section using `same_section`, and the
  persistence model keys tabs by space. Stripping the field is a larger, riskier
  change than making the *active* semantics ignore it, for no user-visible gain.
- **Keep the jump, but jump only on the first click (sticky home).** Rejected: any
  space switch on a global control is the bug, not a feature. Arc doesn't do it,
  and it contradicts ADR-0043 (the favorites strip is fixed chrome that stays put
  across space switches - selecting from it shouldn't itself switch spaces).
- **Make a favorite active in at most one space (move it on each select).**
  Rejected: it would silently pull the favorite off another space's canvas when
  you open it in a new space. Per-space active is what "global, resident in every
  space" actually implies.

## Consequences

- A favorite can be the visible tab of multiple spaces simultaneously; they share
  one browser/web contents (it's one global tab), so all show the same page - the
  expected behavior for a resident pinned site.
- Any code that repairs or restores a space's active pointer must treat favorites
  as valid regardless of their home space - centralized in `TabStore::can_be_active_in`
  so the four restore/repair sites stay consistent. New restore paths must use it.
- Guarded by unit tests in `crates/core/src/tabs.rs`
  (`selecting_favorite_stays_in_current_space`, `deselecting_favorite_acts_on_the_current_space`,
  `deleting_favorite_repairs_every_space_that_showed_it`,
  `favorite_active_in_non_home_space_survives_restore`).
- Refines ADR-0032 (favorites) and ADR-0038 (permanent favorites); complements
  ADR-0043 (favorites strip is fixed chrome across space switches).
