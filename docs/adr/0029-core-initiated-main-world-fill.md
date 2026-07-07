# ADR-0029: Button-driven autofill fills via a core-initiated main-world evaluate

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

The vault quick-access button (P2-34) is a native URL-bar affordance that lists a
site's saved logins and, per entry, offers **Fill / Copy username / Copy
password** — the detection-independent manual path that works when the focus-
driven picker (ADR-0017) can't (a two-step `google.com` sign-in was the trigger).

Copy is trivial (native clipboard + the existing `browser_vault_password`). **Fill
is the new mechanism.** The injection substrate (ADR-0017, `inject.rs`) is
**reply-only**: the page calls a binding (`bindingCalled` event) and the core
answers by evaluating into *that* event's isolated-world `executionContextId`. A
button press has **no originating page message**, so the core must *push* into the
page unprompted — a direction the substrate didn't have. Two sub-decisions:

1. **Which world** — the substrate's isolated world, or the page's main world?
2. **Which detection** — reuse the picker's strict `findFields`, or something new?

## Decision

**Core-initiated fill is a self-contained, lenient fill snippet evaluated in the
main world of the active browser's main frame** (`inject::evaluate_main_world` →
`Runtime.evaluate` with **no `contextId`** → the default context = main frame's
main world). The credential is fetched **only on the explicit gesture** (the
popover selection → `service::login_detail`) and embedded in the snippet as
escaped JS string literals (`autofill::fill_expression`, the same JSON-as-JS
escaping the reply path uses). The snippet is **lenient**: it fills whatever login
fields are present — so it works on an **email-only first step** as well as a full
user+password form.

## Alternatives

- **Isolated-world fill + per-browser `executionContextId` tracking** — rejected.
  It would need the substrate to record each browser's isolated-world context
  (refreshed on every `bindingCalled`, plus an unconditional "ready" ping so a
  page that never triggers a lookup — the email-only step — still registers one,
  plus top-frame disambiguation so a same-origin iframe's context doesn't clobber
  the main frame's). That is real, fragile machinery (context ids change per
  navigation) for **no security gain**: a fill exposes the one value to the page by
  definition (ADR-0017 already accepts this), and the isolated world protects the
  *bridge/lookup channel*, **not** a DOM write — both worlds share the same DOM and
  element prototypes, so the native-setter fill behaves identically in either.
  Main-world evaluate needs none of it (CDP's default context *is* the main frame).
- **Reuse the picker's `DETECT_SCRIPT` `findFields`** — rejected: it requires a
  visible password field before it offers anything, so it would fill nothing on a
  two-step form's first screen — exactly the case this feature exists to fix. The
  button path needs a *lenient* find-and-fill, so it is its own small routine.
- **Drive the fill over the external CDP port (P0-4)** — rejected for the same
  reason as ADR-0027: it drags the consent-gated, off-by-default automation
  surface into an always-on internal action (wrong layer).

## Consequences

- The substrate now has **two directions**: the reply-only page→core→page bridge
  (isolated world, ADR-0017) and this core→page push (main world). They are kept
  distinct in `inject.rs` (`msg_evaluate` vs. `msg_evaluate_default`) so the
  isolated bridge is never confused with the main-world fill.
- **Main-frame only.** Button-Fill does not reach a login form inside a (same-
  origin) iframe; the focus-driven picker (per-frame via `addScriptToEvaluateOnNewDocument`)
  still covers those, and **Copy is host-based so it is unaffected**. Acceptable
  for v1; revisit if sub-frame button-fill is ever needed.
- A tiny `setVal` (native value setter + `input`/`change`) is duplicated between
  the isolated-world detect script and the main-world fill snippet — unavoidable
  (different worlds can't share a function) and accepted; P2-30 (fill-event
  fidelity) will harden both together.
- Secret exposure is unchanged from ADR-0017: a fill inherently reveals the one
  value to the page, and it is gesture-gated (the user picked the entry). The list
  itself stays metadata-only.
