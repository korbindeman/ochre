# ADR-0017: Vault autofill — injected isolated-world content script + in-page picker

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

Autofill is the vault's whole point (ADR-0016). But CEF ships **no** `chrome`-layer password
manager (`rm §5.2` / roadmap §295) — so, exactly like a password-manager *extension*, we build
it: detect login fields, offer matching credentials, fill, and detect submissions to offer to
save. Four sub-problems: form detection, matching, the **picker** (how the user chooses which
login to fill), and save. The picker presentation was a genuine fork with real cross-platform and
security implications.

## Decision

- **Form detection + fill** run in a **content script injected at document-start into an isolated
  world**, over the CDP `Page.addScriptToEvaluateOnNewDocument` path — the *same* injection
  substrate cosmetic ad-block (`spec §11`) and the Tweaks/userscript engine (`spec §20`) need.
  Build that substrate once (backlog P2-14); autofill, cosmetic filtering, and userscripts all ride
  it. The isolated world keeps page JS from reaching the vault bridge.
- **Matching** is in the core: the script reports the page origin, the core runs `vault_lookup(host)`
  (P2-11) and returns credential **metadata only** — never a password until an explicit user gesture.
- **The picker is an in-page inline menu** drawn by the content script, anchored to the focused
  field (1Password-style). Chosen over a native popover.
- **Save is a native banner** (Safari-style) — it is not field-anchored, so it needs no coordinate
  mapping and belongs on the more trustworthy native surface.

## Alternatives

- **Native popover anchored to the field** (Safari/Chrome feel) — rejected: it requires mapping
  field coordinates from the web view into native screen space, tracking scroll/resize, and
  re-implementing that positioning glue **per platform** — logic a second frontend would have to
  redo, against the core-is-shared principle — and it entangles autofill with the CEF `NSView`
  embed. The extra native feel doesn't justify the cost; 1Password's in-page menu is proven. (The
  save banner *is* native precisely because it isn't position-sensitive.)
- **`CefRenderProcessHandler` (render-process V8) for detection** — rejected as the primary path:
  more powerful, but puts logic in the helper (render) process and needs V8 bindings; the CDP
  isolated-world injection gets us what we need with less coupling and reuses the userscript
  substrate. Kept as a fallback if CDP injection proves insufficient.
- **Chromium's built-in password manager** — not an option; CEF omits the `chrome` layer.

## Consequences

- Autofill, cosmetic ad-block, and userscripts **share** the document-start injection + page↔core
  message substrate (P2-14) — one hard thing built once.
- The picker is DOM chrome we own: it must be styled to feel native and is only as spoof-resistant
  as any in-page autofill (the isolated world stops the page forging our *bridge*; a page can still
  draw a look-alike menu — an inherent limit of in-page autofill, same as 1Password).
- Filling a credential necessarily exposes that one value to the page (intrinsic to autofill — the
  page needs it to log in); the vault and bridge stay isolated.
- The full autofill UX is **shell-gated** (needs the web-view integration + FFI + M2 chrome); the
  form-detection + matching **core** (P2-15) is buildable and testable ahead of it.
- Split surfaces: picker = web (position-sensitive), save = native (trust-sensitive) — a deliberate
  division by what each needs.
