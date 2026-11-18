# ADR-0028: Pin CEF to the 148 line - Chromium 149 drops subresource redirects (`ERR_INVALID_ARGUMENT`)

- **Status:** Superseded by ADR-0074
- **Date:** 2026-07-07

## Context

The signed-in Google profile picture (and, it turns out, any redirect-based subresource) failed to
load with `net::ERR_INVALID_ARGUMENT`. Diagnosed against the then-pinned `cef 149.3.0+149.0.6`
(Chromium 149.0.7827):

- **The request issued *after* any subresource HTTP 3xx redirect fails with `ERR_INVALID_ARGUMENT`.**
  Direct (non-redirect) subresource requests load fine. The avatar is served via the One-Google-Bar
  `lh3.googleusercontent.com/rd-ogw/…` **redirect** endpoint, so it hits this directly.
- **Proven engine-level, not our code.** Reproduced in an isolated instance over CDP (see
  [[cdp-verification-harness]] / [[verify-app-isolated-home]]) with a *real markup* `<img>` and with
  an *external, well-formed* redirect (Wikimedia `Special:FilePath`). Persists with our adblock
  `ResourceRequestHandler` off **and** with the entire `RequestHandler` removed; under both
  `RuntimeStyle::DEFAULT` and `ALLOY`. Not cross-origin-specific (same-origin redirects fail too),
  not URL-length. `149.3.0` is the newest published cef-rs, so there is **no forward bump** available.
- **Impact:** any redirect-based subresource (avatars, some tracking pixels, image-CDN shorteners)
  silently breaks; most images are served directly, so the breakage is mostly invisible - which is
  why only the profile picture surfaced it.

The `cef`/`cef-dll-sys` crates publish a full version history (146.x–148.x still available), so
downgrading is possible; the question was whether an older line follows redirects.

## Decision

**Pin `cef` and `cef-dll-sys` to `148.4.0+148.0.10`** (the newest 148 line) in
`[workspace.dependencies]`. Verified end-to-end: on 148 the full redirect chain is followed and the
image loads (`responseReceived 200 → loadingFinished`), including the exact avatar-shape case that
returned `ERR_INVALID_ARGUMENT` on 149. The caret `148.4.0` keeps us on the 148 line and **below**
the 149 regression. **Do not bump to 149 until the regression is fixed upstream.**

## Alternatives

- **In-app workaround - a resource handler that detects 3xx and manually follows the redirect**
  (fetch the `Location`, stream the response back) - rejected: substantial, fragile code on the
  request path (HTTP methods, header/cookie forwarding, CORS, redirect chains) to paper over an
  upstream bug, against "stay lean" - and the handler is *provably* not the cause, so re-adding one
  to work around it is backwards.
- **Stay on 149, report upstream, wait** - rejected: leaves redirect-based images broken in a
  shipping surface indefinitely.
- **Bump forward to a fixed cef-rs** - not possible: `149.3.0` is the latest published release.

## Consequences

- Engine is Chromium **148.0.7778** instead of 149.0.7827 - one line older; negligible on the
  modern-web / DRM axis (see [[browser-roadmap-and-engine-decision]]), and it restores redirect
  following.
- The bisect also **localized the regression to the 149 line** (148 works, 149 doesn't) - so it is a
  CEF/Chromium-149 regression, not cef-rs binding glue. A clean upstream report is therefore possible;
  minimal repro is a page with `<img src="<a-URL-that-302s>">` (direct loads, redirected fails).
  Filing it upstream (tauri-apps/cef-rs → CEF) is an open follow-up, not yet done.
- A CEF version change must clear the focus-routing regression guard (ADR-0018): re-ran
  `just focus-selftest` on 148 - passes. `just verify` green on 148.
- **Added a standing guard: `just redirect-selftest`** (`crates/core/src/redirect_selftest.rs`).
  Nothing in the unit suite exercised a real subresource redirect, so `just verify` passed on
  *both* the broken (149) and fixed (148) engine - the regression was invisible to CI. The self-test
  is hermetic (an in-process loopback server whose `<img>` loads through a same-origin 302, beaconing
  the outcome) and exits non-zero if the redirect isn't followed - the network analogue of the focus
  self-test. **Re-run it after any CEF bump**, so a future attempt to move off this pin fails loudly
  if the regression is still present.
- Docs that mention "CEF 149" (e.g. ADR-0021, the spike notes) now name an engine older than the
  pin in spots; those decisions are unaffected by the version (ADR-0021's separate-DevTools-window
  choice still holds). **Revisit this pin when upstream ships a fix** - bump to the first 149+/150
  line that follows subresource redirects and supersede this ADR.
