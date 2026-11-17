# ADR-0027: Cross-origin iframe (OOPIF) injection is blocked by CEF's DevTools observer

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

The injection substrate (ADR-0017, `crates/core/src/inject.rs`) drives CDP
in-process via `CefBrowserHost::SendDevToolsMessage` + a
`CefDevToolsMessageObserver`. It reaches the **main frame and every same-origin
(same-process) iframe** for free - `Page.addScriptToEvaluateOnNewDocument` and
`Runtime.addBinding` apply per-frame, and `Runtime.bindingCalled` events carry the
originating `executionContextId` so replies target the right frame. This is
**verified**: a same-origin child iframe's `.ad-banner` is cosmetically hidden.

**Cross-origin iframes (OOPIFs)** run in a separate renderer process and are a
separate DevTools *target*, not covered by the root session's injection. Cookie-
consent Tier B (ADR-0026) needs them (some CMPs render in cross-origin iframes),
so P2-24 set out to add all-frames coverage via `Target.setAutoAttach`.

## Decision

**Cross-origin iframe injection is not achievable through CEF 149's internal
DevTools interface. Ship same-origin coverage; defer OOPIFs to a render-process
path (P2-24), and do not carry non-working auto-attach code.**

The investigation (kept here so it isn't repeated):

1. `Target.setAutoAttach{flatten:true, waitForDebuggerOnStart:true}` **works** -
   `Target.attachedToTarget` arrives for the OOPIF with its child `sessionId`.
2. Messages tagged with that `sessionId` **reach the child** - `Page.enable`,
   `Runtime.enable`, `Runtime.addBinding`, `addScriptToEvaluateOnNewDocument` all
   return their **results** through the observer.
3. **But no *events* ever come back from the child session.** A deliberate
   `Runtime.evaluate("console.log(...)")` on the child produced its *result* but
   **no `Runtime.consoleAPICalled` event** - and likewise no
   `Runtime.executionContextCreated`, no `Runtime.bindingCalled`.

So CEF's `CefDevToolsMessageObserver` relays flatten-mode child-session **method
results but not events**. The substrate's page→core edge *is* an event
(`bindingCalled`), so it can never arrive from an OOPIF - the channel is
one-directional there and unusable.

## Alternatives

- **Auto-attach via the internal observer** (the P2-24 attempt) - rejected: proven
  non-functional above. Carrying it would add ~10 no-op CDP round-trips per OOPIF
  for zero effect - dead code, removed.
- **Drive OOPIF targets over the external CDP port** (P0-4), where events *do*
  flow - rejected: it drags the external, consent-gated automation surface into
  the always-on internal substrate (wrong layer; also requires the port enabled).
- **`CefRenderProcessHandler` (render-process V8)** - the real fix. Each renderer
  (including every OOPIF's) sets up its own isolated-world injection + an IPC
  channel to the browser process, sidestepping the DevTools observer entirely.
  ADR-0017 already named this as the fallback "if CDP injection proves
  insufficient." It is a larger architectural piece (render-process handler + V8
  context bindings + browser↔render IPC), so it is **deferred to P2-24**, not built
  under time pressure here.
- **Wait for a CEF that relays child-session events** - possible but out of our
  control; revisit on a CEF bump.

## Consequences

- **Same-origin iframes are covered and shipping** (autofill, cosmetic ad-block,
  cookie-consent all reach them). Cross-origin iframe CMPs / ad frames are **not**
  handled until the render-process path lands.
- The substrate stays lean: the failed auto-attach code is reverted; `inject.rs`
  is the proven main-frame-plus-same-origin implementation.
- **P2-24 is reframed**: not "all-frames via auto-attach" (blocked) but "OOPIF
  injection via a render-process handler" - a distinct, larger effort, still the
  prerequisite for full cookie-consent (ADR-0026) and sub-frame cosmetic coverage.
- Documented so the next agent doesn't re-run the same three-step probe.
