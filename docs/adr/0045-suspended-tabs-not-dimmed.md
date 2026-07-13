# ADR-0045: Suspended tabs render identically — no sidebar dimming

- **Status:** Accepted
- **Date:** 2026-07-13

## Context

ADR-0013 (lazy restore) and ADR-0033 (runtime suspension) both anticipated an Arc-style
**sidebar dimming** for tabs whose CEF browser has been discarded: ADR-0013 §Consequences
called it "possible later by exposing hydration state," and ADR-0033 added `hydrated: bool`
to the structure snapshot expressly "for sidebar dimming." That shipped as a 55% opacity on
the tab row when `isHydrated == false`.

In use the dimming read as a defect, not a status. Suspension is an invisible memory
optimization — a suspended tab reloads transparently from its saved URL on the next click,
indistinguishable from a warm tab to the user. Graying the row implied the tab was broken,
stale, or unavailable, none of which is true. It also flickered: a restored-at-launch or
grace-window-suspended tab would dim, then snap to full opacity the instant it was shown,
drawing the eye to churn the user shouldn't have to think about.

## Decision

Suspended (unhydrated) tabs render in the sidebar **identically to loaded tabs** — no opacity
change, no other affordance. The `.opacity(...)` on `TabRow` is removed, and with it the
Swift-side `isHydrated` projection (the field, its `TabSuspended`/snapshot assignments, and the
snapshot's decoded `hydrated` key) — that state existed **solely** to drive the dimming.

The suspension mechanism itself is untouched: the core still discards browsers per ADR-0033,
still emits `TabSuspended` (type 8), and the host still detaches the CEF `NSView` on it via
`suspendedTabId`. The core structure snapshot keeps emitting `hydrated: bool` (a `browser.is_some()`
projection still asserted by the import self-test); the Swift layer simply no longer reads it.

## Alternatives

- **Keep the dimming** — rejected: it communicates "unavailable/stale," which is false for a
  tab that reloads transparently on click; the launch/grace-window flicker made it net-negative.
- **A subtler suspended affordance** (e.g. a tiny "zzz" glyph, a hairline, a muted favicon) —
  rejected for v1: suspension is deliberately invisible to the user; any marker re-introduces the
  same "this tab is different/worse" read for zero user benefit. Re-openable later if a concrete
  need appears — the core still exposes `hydrated`.
- **Remove `hydrated` from the core snapshot too** — rejected: out of scope and still consumed
  by `import`'s self-test as an observable assertion that imported tabs come back suspended.

## Consequences

- The sidebar no longer reveals which tabs are warm vs. suspended — intended; that's an
  implementation detail, not user-facing status.
- `hydrated` in the structure snapshot now has no frontend consumer; it stays as a cheap,
  already-tested projection and a re-entry point should a future suspended affordance be wanted.
- Reverses the "dimming" direction pre-committed in ADR-0013 and ADR-0033; those ADRs' core
  decisions (lazy restore, runtime suspension) stand unchanged.
