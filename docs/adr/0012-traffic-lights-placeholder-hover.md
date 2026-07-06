# ADR-0012: Traffic lights — custom-drawn in the sidebar, native buttons hidden

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

The Arc-style sidebar wants the macOS traffic lights to read as part of the sidebar: at its top,
sliding/fading with it, and gone when it's collapsed so window controls never strand over
full-bleed web content (style §3).

The system buttons, however, are `NSWindow` chrome owned by AppKit, which re-lays them out —
resetting **both** position and visibility to its own defaults — on a wide family of triggers:
live resize, key/main (focus) changes, (de)miniaturize, fullscreen transitions, and internal
titlebar rebuilds. Three implementations that kept the *real* buttons and tried to place/hide
them all failed the same way — worst on the macOS 26/27 beta, whose titlebar internals differ:

1. **Delegate-callback chasing** — reposition + fade from `windowDidResize`/fullscreen callbacks
   and sidebar-state publishers. Drifted whenever AppKit reset from a trigger the callbacks
   didn't cover (lights snapping to the OS corner, reappearing over a collapsed sidebar).
2. **Re-assert on relayout** — observe the close button's frame changes and re-apply from one
   idempotent writer. Still inverted in practice: animated visibility raced AppKit's resets and
   the sidebar publishers (hidden when the floating sidebar was up, visible when the sidebar was
   gone).
3. **Placeholder dots + hover-revealed real buttons** — dots as sidebar content (animate for
   free), real buttons parked on top only on hover. On the beta OS the overlaid real buttons
   landed in the wrong place and didn't track the dots — the coordinate math against AppKit's
   titlebar simply doesn't hold there.

The through-line: **any design that positions or shows/hides AppKit's own titlebar buttons is at
the mercy of an owner that re-lays them out at unpredictable times.**

## Decision

Stop using the system buttons while windowed. **Draw the three traffic lights ourselves** as
ordinary SwiftUI content in the sidebar's top row (`TrafficLights` in `TrafficLights.swift`), and
**hide the real `NSWindow` buttons** (`AppDelegate.setNativeButtonsHidden`).

- Because the lights are sidebar content, they inherit the sidebar's slide/fade animations and
  are always positioned correctly — zero coordinate math, zero AppKit relayout to fight.
- Fidelity matches native: exact fill colors, a 0.5pt darker ring, the window-inactive gray state
  (via a dynamic `NSColor`), and hover-revealed glyphs (× / − / the green fullscreen triangles).
  Hover is a property of the cluster, so all three glyphs reveal together, as on macOS.
- Actions route to the real window: red → `performClose:` (so it still hits the graceful CEF
  teardown in `windowShouldClose`), yellow → `miniaturize:`, green → `toggleFullScreen:`.
- **Fullscreen defers to the OS.** On enter we un-hide the native buttons (the auto-hiding
  titlebar presents them) and the custom set steps aside via `BrowserState.isFullscreen`; on exit
  we re-hide them. The native buttons are also re-hidden on key/main changes, since AppKit can
  restore them on its own relayout — this is the one remaining AppKit touch-point, but it only
  toggles `isHidden` (robust), never position (the fragile part).

## Alternatives

- **Keep the real buttons, place/hide them** (the three approaches above) — rejected empirically,
  repeatedly, and worst on the target beta OS. This is a dead end; do not re-attempt it.
- **Reparent the real buttons into our own view** (remove from the titlebar, `addSubview` into a
  sidebar container) — would keep native fidelity for free, but reparenting standard window
  buttons is finicky (AppKit can re-adopt them) and was the riskier bet on an already-unstable
  beta. Not chosen; revisit only if custom-draw fidelity proves insufficient.
- **`NSTitlebarAccessoryViewController`** — rejected: accessories add views *around* the standard
  buttons; they don't reposition or hide the buttons themselves, so they solve neither problem.

## Consequences

- We own appearance and behavior, and must track native changes across macOS releases. The
  common case (colors, inactive gray, hover glyphs, close/minimize/fullscreen) is covered; the
  exotic bits — option-key alternate glyphs, VoiceOver parity, the green zoom-vs-fullscreen
  nuance — are deliberately deferred (a `later` polish item) rather than shipped half-done.
- The lights are absent when the sidebar is fully collapsed (they *are* the sidebar), matching
  the intended Arc behavior — reveal the floating sidebar (hover the left edge) or use ⌘W / the
  menu to close. A documented deviation from host-OS window-control placement (style §3).
- One small AppKit dependency remains: re-hiding the native buttons on key/main/fullscreen
  changes. It is visibility-only (`isHidden`), which is stable, unlike the positioning that made
  the prior approaches fragile.
- Windows (future frontend) is untouched — caption buttons there are a parity *Idiom* row; this
  is macOS-only chrome.
