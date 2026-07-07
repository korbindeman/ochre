# ADR-0019: Sidebar glass — standard sidebar material, beside content (not lensing over the web view)

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

`mvp1.md` M2 names one thing to validate: the target macOS 26 Safari look is *content full-bleed with
the slim glass chrome floating over it, glass lensing live page content* — and "floating a glass view
over the CEF `NSView` is the native-view-layering seam that can bite. Validate in M2. Fallback: content
flush beside a slim glass sidebar." `style §54` carries the same fork. This ADR records the validation
outcome so the seam isn't re-explored.

The deciding fact is architectural, not cosmetic: **an in-window translucent/glass material samples the
pixels behind it, but CEF renders the web view as a GPU-hosted remote layer that in-window blur cannot
sample reliably** (already discovered and commented on the `FloatingSidebar`, which is solid on purpose
for exactly this reason). So a glass sidebar composited *over* the CEF view has nothing valid to lens —
it would sample garbage or nothing where the page is.

## Decision

The sidebar sits **flush beside** the content region (the documented fallback), and its glass is the
**standard macOS sidebar material** — `NSVisualEffectView(.sidebar, .behindWindow)`, which macOS 26
renders in the Liquid Glass style automatically, sampling the desktop behind the *window* (valid, since
the sidebar strip is not over the CEF view). Discrete Liquid Glass (`.glassEffect`) is reserved for the
**floating** chrome that legitimately sits over content — the command bar, find bar, and history
capsules — where each is a small element with its own solid-enough backing, not a blur over live web
pixels.

Concretely this means: **do not** put an `NSGlassEffectView` / `.glassEffect` backdrop on the sidebar.
That would both (a) try to lens content it can't sample and (b) violate `style §55` ("glass on the top
functional layer only — never as a stacked backdrop"). The sidebar's translucency comes from the window
material; glass lives on the floating controls.

## Alternatives

- **Glass sidebar lensing the web view (the mvp's aspirational target)** — rejected. In-window glass
  can't sample CEF's GPU remote layer, so there is nothing to lens; the result is unreliable/garbage
  backdrop. This is the seam M2 was told to validate, and it does bite. Would require compositing the
  web content into a layer the window server can sample (out of CEF's windowed-rendering model) or
  offscreen/`OSR` rendering (a different, heavier engine-hosting mode we explicitly avoid).
- **`NSGlassEffectView` as the full-height sidebar backing** — rejected. Even setting aside the
  sampling problem, macOS 26 restyles the standard `.sidebar` material to Liquid Glass for free, and a
  full-height glass *backdrop* is what `style §55` prohibits (glass is the top functional layer, not a
  stacked backing). `NSGlassEffectView` is for discrete floating elements, which we already use it for.
- **Offscreen-render (OSR) the web content so glass can sample it** — rejected for MVP. OSR trades away
  windowed-rendering performance/fidelity and complicates the whole hosting model for one cosmetic
  effect; not worth it (revisit only if over-content lensing becomes a hard requirement).

## Consequences

- The MVP sidebar look is "slim glass chrome beside full-bleed content" — slim and content-first, just
  without over-content lensing. Consistent with `style §3/§54/§55`.
- The signature Liquid Glass *is* present where it reads best and composites correctly: the floating
  command bar, find bar, and history capsules.
- If a future platform/engine path makes the web content sampleable in-window (compositing change, or a
  deliberate OSR mode), over-content lensing can be revisited — this ADR would be superseded, not edited.
- `style §54`'s "validate in M2" is now resolved to the fallback; the style guide is updated to point
  here instead of leaving the fork open.
