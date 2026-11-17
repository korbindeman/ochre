# ADR-0023: Chrome icon buttons share one `IconButtonStyle` (hover + press feedback)

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

The chrome's custom controls are all `.buttonStyle(.plain)` - we opt out of the
system chrome look (§13) and paint our own quiet rounded highlight. Each button
hand-rolled that highlight: a `@State hovering`, an `.onHover`, and a
`.background(RoundedRectangle().fill(.quaternary).opacity(hovering ? x : 0))`,
later given a shared `hoverFade` cross-fade (`Motion.swift`). That left two gaps:

- **Drift and omission.** Corner radii and hover opacities were copied per site
  and diverged (0.45 / 0.5 / 0.7), and some buttons never got the treatment at
  all - most visibly the URL chip's **copy** and **shield** buttons, which showed
  a glyph-color change but no fill, so they didn't read as buttons "like the
  rest." A hand-rolled convention is only as consistent as the next copy-paste.
- **No pressed state.** *Not one* custom button deepened on mouse-down. Native
  macOS / Arc toolbar buttons give two-stage feedback - a fill on hover, a deeper
  fill on press - and `hoverFade` (an animation on a `Bool`) structurally can't
  express the press half; the button's own gesture (`isPressed`) has to.

"Every interactive control shows hover / focus / active / disabled correctly"
(§10) was aspirational, not enforced. Making it real means one place that owns the
feedback, not a rule each author re-implements.

## Decision

A single **`IconButtonStyle: ButtonStyle`** (`IconButton.swift`) owns the chrome
icon-button feedback: a `.quaternary` rounded fill that appears on hover and
**deepens on press** (read from `configuration.isPressed`), animated with the
shared `Animation.hover` curve and gated on Reduce Motion. Parameters cover the
only legitimate per-site differences:

- `cornerRadius` - matches the button's container.
- `isActive` - keeps the fill lit at rest for a persistently-selected control
  (the active space pill, the open Passwords toggle).
- `emphasizesLabel` - brightens a plain glyph secondary → primary on hover/press
  and dims it to quaternary when disabled; turned **off** for buttons whose color
  already encodes state (copy → green, shield → tint), which keep their own
  `foregroundStyle` and take only the fill.

Every custom icon button (nav back/forward/reload/sidebar, tab close, New Tab
row, empty-state, space pills + switcher, URL copy + shield, find-bar controls,
history/vault row actions) adopts it. `hoverFade` stays as the primitive for the
tap-gesture **rows** and the URL chip (which are selection surfaces, not
`Button`s); `IconButtonStyle` uses the same curve so the two move alike. Codified
in `style-guide.md §17`.

## Alternatives

- **Add the missing hover fills inline, keep hand-rolling** - rejected: it's the
  status quo that drifted and skipped buttons, and it can't cleanly add the
  pressed state (each button would need to thread `isPressed` by hand). The whole
  point is to stop re-implementing the convention.
- **Fold everything (rows included) into the style** - rejected: the tab/history/
  vault rows aren't `Button`s (they use `onTapGesture` so a nested close/reveal
  button's tap stays unambiguous), and the active-tab row carries a sliding
  `matchedGeometryEffect` pill. Forcing them through a `ButtonStyle` would fight
  both. Rows keep `hoverFade`; only real buttons take the style.
- **Style also owns foreground for every button (no `emphasizesLabel`)** -
  rejected: state-colored glyphs (copy's green, the shield's tint) would be
  overridden or need fragile `foregroundStyle`-precedence gymnastics. An explicit
  opt-out is simpler and predictable - the style never touches a button's color
  unless asked.
- **Use the system `.bordered`/`.borderless` button styles** - rejected: they
  impose the platform's control chrome, which §13 explicitly rejects for our
  slim, quiet chrome; we want native *behavior* (hover/press/Reduce-Motion), not
  the native rendered button.

## Consequences

- One knob for chrome button feel: change the fill ramp, curve, or press depth in
  `IconButtonStyle` and every button follows. New buttons get correct, uniform
  hover + press + disabled + Reduce-Motion behavior by adopting one modifier -
  the §10 "shows all states" rule is now the path of least resistance.
- Two deliberate minor visual shifts fall out of unifying: the nav buttons' hover
  fill eases from 0.7 → the shared ramp, and the open Passwords toggle now reads
  as a full "selected" fill (like a space pill) rather than a half-lit one.
- Action feedback generalizes with it: the vault **copy** button now flashes a
  checkmark like the URL chip's Copy Link, per the §17 "actions confirm within a
  frame" rule.
- The split must be kept honest: a *button* uses `IconButtonStyle`; a *tap-gesture
  row* uses `hoverFade`. A new icon button that hand-rolls a hover `.background`
  is a regression to reject in review.
- Focus rings / keyboard-activation feedback are unchanged (still the system's on
  the underlying `Button`); this ADR governs the pointer hover/press fill only.
