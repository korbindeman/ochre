# ADR-0054: Pinned favorite selection uses a radial bloom beneath glass

- **Status:** Superseded by [ADR-0055](0055-pinned-favorite-inset-edge-bloom.md)
- **Date:** 2026-08-13

## Context

ADR-0053 replaced the favorite strip's traveling active ring with a small system-
accent dot. The dot removed the conflicting border and motion, but it read as a
detached status badge rather than a selected state intrinsic to Liquid Glass.

Favorites already have a dominant site color extracted from their favicon
(ADR-0046). Selection can reuse that identity while keeping the separate glass
controls stable.

## Decision

The active favorite renders a saturated radial bloom beneath its Liquid Glass
surface, centered behind the favicon.

- The bloom uses the favicon's dominant color, with the system accent as the
  fallback for monochrome icons.
- A strong center falls to transparent before the tile edge, preserving the native
  glass boundary instead of painting a selected slab.
- The gradient is rendered behind `.glassEffect`, allowing the material to refract
  and scatter its color.
- Selection animates opacity and scale locally with an interruptible SwiftUI spring.
  Reduce Motion applies the state immediately.
- The glass shape, favicon, and pointer interaction do not move. Nothing travels
  between favorites.

The active favorite retains the accessibility selected trait.

## Alternatives

- **Small accent dot (ADR-0053).** Rejected after visual testing: it read as a
  separate status marker, not as state carried by the glass control.
- **Full-tile color tint.** Rejected: an even fill recreates the opaque-looking card
  that prompted the redesign and suppresses the material's depth.
- **Animated gradient shared between tiles.** Rejected: cross-tile travel makes
  independent controls appear to share one movable surface.

## Consequences

- Selection is expressed through the existing site identity and glass material.
- The radial falloff keeps the tile edge quiet while giving the favicon a clear
  selected glow.
- Rapid selection changes retarget the local spring without queuing animation.
