# ADR-0007: Cross-frontend parity tracked as a status-only ledger classified by the consistency contract

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

The project targets multiple native frontends (macOS now; Windows, then Linux) over one shared Rust
core, with a stated goal of **complete feature parity and consistent UI that still conforms to each
platform's idiosyncrasies**. `style-guide §2` already defines the *contract* — the hard split between
what must be identical everywhere and what must be native to each platform — and `spec §6` maps each
Arc behavior onto native widgets. But nothing **tracks** the contract: there was no place that
enumerates every user-facing capability and records, per frontend, whether it's built and whether it
honors the split. With a second frontend approaching, "are we at parity, and where are the gaps?"
had no answerable artifact, and deliberate platform *differences* (native materials, window controls)
risked being mistaken for gaps — or parity gaps mistaken for intentional differences.

## Decision

Add **`docs/parity.md`** — a single status-tracked ledger at a new *coverage/conformance* altitude,
distinct from the temporal backlog and the design docs. Structure:

- **One matrix**, grouped by area, rows = capabilities, columns = frontends (macOS / Windows / Linux).
- **A class column** applying `style §2` to each row: **Inv** (must be behaviorally identical — a
  divergence is a bug) vs. **Idiom** (must adapt per platform — uniformity is the bug; parity means
  "the adaptation exists and follows the contract"). The class column is what carries the
  "conforms to platform idiosyncrasies" nuance without a second table.
- **Status only** (`✅ / 🚧 / ⬜ / ➖`). Behavior stays defined in `style`/`spec`, linked per row;
  the ledger never re-explains it — one source of truth per fact.
- **Kept honest by ritual:** flipping a cell is part of a capability's "done"; a real gap on a
  shipping frontend is filed as a `backlog.md` `next` row. The ledger is the coverage map; the
  backlog is the queue.

## Alternatives

- **Two matrices (functional parity vs. UX consistency)** — rejected: doubles the surface to
  maintain and cross-check, and splits one capability's story across two tables. The single `Inv`/
  `Idiom` class column captures both axes in one grid.
- **Capability-centric entries (prose + per-platform block per feature)** — rejected: richest
  per-feature detail but loses the bird's-eye gap view, which is the entire reason to track parity.
- **Extend `style-guide §2` in place** — rejected: the style guide defines the *rule* and should
  stay stable; a status-tracked ledger is a different altitude (coverage, not definition) and would
  bloat and churn the guide.
- **Fold parity gaps into the backlog only** — rejected: the backlog is *temporal* (what's next),
  not a *coverage grid*. It can't answer "does feature X exist on all three frontends," and would
  balloon if it carried the full cross-product.
- **Machine-checked conformance now (generate rows from the FFI manifest / conformance tests)** —
  deferred, not rejected. It's the durable long-term anti-drift answer, but overkill with one
  frontend and an embed-only FFI today. Noted in `parity.md` as *Future*; revisit when a second
  frontend and a richer FFI surface exist.

## Consequences

- One artifact answers "where are we on parity" and doubles as the **build-spec** for each new
  frontend — every `⬜` in its column is a to-do.
- **Intentional divergences are now documented** (the `Idiom` rows), so a deliberate platform
  difference isn't "fixed" into uniformity, and an `Inv` divergence is unambiguously a bug.
- New maintenance obligation: the ledger must be updated as capabilities land. Mitigated by wiring the
  cell-flip into the "done" ritual — the `steer` skill ("Keeping the trackers honest" + Mode-1 close-the-
  loop) and `CLAUDE.md`'s steering section both make syncing `parity.md` part of marking work done, like
  the backlog. Left unmaintained it still drifts — the standing fix is to eventually anchor core-feature
  rows to the FFI manifest so they can't diverge from the code.
- Adds a doc to the set; wired into `docs/README.md` (index + cross-ref prefix `parity`), the `steer`
  skill, `CLAUDE.md`, and a bidirectional pointer from `style-guide §2` (the contract → its tracker).
