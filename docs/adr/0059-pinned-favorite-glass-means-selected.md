# ADR-0059: Pinned favorite selection is the glass chip

- **Status:** Accepted, supersedes [ADR-0055](0055-pinned-favorite-inset-edge-bloom.md)
- **Date:** 2026-08-14

## Context

Favorites are discrete controls in the strip below the URL bar. Vertical tab
rows already use Liquid Glass as the selected state: idle rows have no glass, the
active row is the glass pill.

Favorites had spent that language on furniture. Every tile was a glass button at
rest, so selection had to invent a second paint job: a solid site-color ring
(ADR-0046), then a local accent dot (ADR-0053), then a radial bloom (ADR-0054),
then an inset edge bloom (ADR-0055).

The ring competed with the material's own specular rim. The bloom (a fat blurred
stroke under the glass) was eaten by the frosting and read as a muddy halo. Both
failed for the same reason: they drew on a control that had already used glass
for presence.

Idle tiles still need a body (ADR-0046): a 18pt favicon in an empty slot
dissolves into the sidebar. Presence and selection have to be different
materials.

## Decision

Glass means selected, matching the tab list.

- **Idle** favorites are a quiet `.quaternary` rounded fill. Slot size and
  favicon stay the same. Hover deepens the fill (`hoverFade`). No glass, no
  site-color wash.
- **Active** favorites are the Liquid Glass chip, site-color `.tint()` from the
  favicon's dominant hue (ADR-0046 extraction). Monochrome icons get untinted
  glass. `.interactive()` carries hover and press on the selected tile.
- Selection animates locally with an interruptible SwiftUI spring. Reduce Motion
  applies the state immediately. Nothing travels between favorites.
- Glass is mounted only while selected. A zero-opacity glass view still lenses.

The active favorite retains the accessibility selected trait.

## Alternatives

- **Keep every tile as glass; add an inset hairline on the active one.**
  Rejected: a drawn rim still sits in the same visual register as the native
  glass edge, even when inset. The user ruled it out.
- **Keep iterating the inset edge bloom (ADR-0055).** Rejected: one fat blurred
  stroke cannot produce both a sharp caustic and soft scatter, and the glass
  then frosts whatever remains.
- **Tint idle tiles with site color and reserve glass for selection.** Rejected:
  identity at rest recreates a colored card next to the selected glass, so the
  strip stays busy and selection is less of a material change.
- **Idle tiles with no fill, favicon only.** Rejected: ADR-0046's invisible-at-rest
  problem returns, especially for monochrome glyphs.
- **Traveling glass via `matchedGeometryEffect`.** Already rejected (ADR-0053):
  independent controls would appear to share one movable surface. The tab list
  can slide a pill because its rows are one list; favorites are separate
  buttons.

## Consequences

- Favorites and vertical tabs share one selection language: glass is the current
  thing.
- Idle presence is the quaternary floor, not glass. The strip is quieter at rest.
  Site color appears only on the selected tile.
- ADR-0046's extraction and saturation gate stay. Its persistent *tinted* rest
  chip and active ring do not; presence is now the neutral fill.
- Rapid selection changes retarget the local spring without queuing animation.
