# ADR-0055: Pinned favorite selection blooms inward from the rounded edge

- **Status:** Superseded by [ADR-0059](0059-pinned-favorite-glass-means-selected.md)
- **Date:** 2026-08-13

## Context

ADR-0054 placed the strongest selection color behind the favicon and faded it
outward. Visual testing showed the intended treatment was the opposite: color
should originate at the tile's border and diffuse inward, leaving the center
quieter.

A circular radial gradient cannot keep a consistent distance from every edge of a
wide rounded rectangle. It produces an oval falloff and does not follow the corner
radius.

## Decision

The active favorite renders a blurred inset stroke beneath its Liquid Glass
surface. The stroke uses the same 10-point rounded rectangle as the glass shape.

- The stroke uses the favicon's saturated dominant color, with the system accent
  as the fallback for monochrome icons.
- The stroke is clipped to the tile, so its rounded outer edge stays exact while
  its blur diffuses only inward.
- A low-opacity color floor keeps the center connected to the edge glow without
  becoming a flat selected fill.
- Selection animates the inset width, blur radius, and opacity with an interruptible
  SwiftUI spring. Reduce Motion applies the state immediately.
- The glass shape, favicon, and pointer interaction remain fixed. Nothing travels
  between favorites.

The active favorite retains the accessibility selected trait.

## Alternatives

- **Center-origin radial bloom (ADR-0054).** Rejected after visual testing: its
  emphasis was spatially inverted.
- **Reversed circular radial gradient.** Rejected: it cannot maintain even color at
  the straight edges and properly rounded corners of a wide tile.
- **Hard colored border.** Rejected: it competes with the native glass boundary
  instead of reading as light refracted through the material.

## Consequences

- Selection color is strongest near every rounded edge and softens toward the
  favicon.
- The shape remains correct across one-, two-, and three-column favorite layouts.
- Rapid selection changes retarget the local spring without queuing animation.
