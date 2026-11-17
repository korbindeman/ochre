# ADR-0053: Pinned favorite selection uses a local accent indicator

- **Status:** Superseded by [ADR-0054](0054-pinned-favorite-radial-selection-bloom.md)
- **Date:** 2026-08-13

## Context

Pinned favorites are discrete Liquid Glass controls with a low favicon-color tint
(ADR-0046). The active state added a stronger favicon-colored wash and a 1.5-point
ring that used `matchedGeometryEffect` to slide between favorites.

That treatment introduced a second material language on top of the glass. The
selected favorite became an opaque-looking colored tile, and the traveling ring
made stable, separate controls appear to share one movable surface. Apple uses
foreground color to identify a selected tab item and reserves tinted glass for
prominent actions. A branded favicon cannot itself take the system accent color
without losing its identity.

## Decision

Keep every favorite's glass geometry and favicon tint stable across selection.
Show the active favorite with a 4-point system-accent dot inside the control,
centered below the favicon.

- The indicator changes opacity in place using the shared 120 ms hover curve.
- Reduce Motion removes the fade.
- The selected view exposes the accessibility selected trait.
- Favorite selection has no active-only wash, stroke, matched geometry, or motion
  between controls.

The vertical tab list keeps its sliding row selection pill. This decision applies
only to the favorite strip, whose items are independent glass controls rather than
rows in one list.

## Alternatives

- **Tint the selected glass with the system accent.** Rejected: it keeps selection
  in the background and still makes the selected tile read as a prominent action.
- **Recolor the selected favicon.** Rejected: favicons are branded, often
  multicolor artwork whose identity depends on its original colors.
- **Keep the ring and remove only its slide.** Rejected: the extra border still
  competes with the native edge and lighting already drawn by Liquid Glass.
- **Use a checkmark badge.** Rejected: a checkmark implies completion or a
  multi-select choice, not the page currently shown on the canvas.

## Consequences

- Selection stays visible without repainting the tile or competing with glass.
- Switching favorites produces only a short local cross-fade, so rapid changes are
  interruptible and no geometry travels across unrelated controls.
- The system accent color carries selection while each favicon keeps its brand
  color and its low-intensity identity tint.
