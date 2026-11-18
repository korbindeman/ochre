# ADR-0065: Every fix adds a mechanical guard; core invariants get property tests

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

Agents do most of the work in this repo. Many lessons live as prose: CLAUDE.md gotchas, crash
post-mortems, ADR consequences. Prose works only when an agent reads it at the right moment, and the
same classes of bug have come back (SwiftUI overlays on gesture trees, key-event-only properties on
non-key events, handler splitting per tab). `just emdash-check` shows the alternative: a rule a
machine enforces.

With friends relying on Ochre (ADR-0063), the state that holds their data (tab and Space tree,
Pinned URLs, session save and restore, Arc import) must not break.

## Decision

- **A bug fix or crash post-mortem is not done until it adds a mechanical guard** that would have
  caught it: a clippy or dylint lint, a source check in `just verify` (like `emdash-check`), a unit or
  property test, or a selftest. If no guard is possible, the post-mortem says why.
- **Core invariants get property tests first** (proptest): the Favorite/Pinned/Open tree and its
  operations in `tabs.rs`, session round-trips in `session.rs`, Pinned URL rules (ADR-0056/0057), and
  Arc import. Model checking (TLA+, Kani) is a later option for specific state machines.
- Existing prose gotchas are converted to guards where possible.
- The repo stays on git. Aether is not ready yet; revisit when it is.

## Alternatives

- **Add guards when it seems worth it** - rejected: that is the current state, and lessons stay prose.
- **Model checking first** - rejected for now: property tests on the Rust core catch the likely
  data-loss bugs at lower cost; model checking stays available for one machine at a time.

## Consequences

- Fixes cost a little more each; repeat bugs should drop.
- `just verify` grows; keep each check fast.
- CLAUDE.md and the `crash` skill state the rule.
