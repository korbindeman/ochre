# ADR-0047: Windows is a parallel frontend track, not gated behind Phase 4

- **Status:** Accepted
- **Date:** 2026-07-13

## Context

The roadmap's five phases (`rm §5`) track the **public-product bar** — "how complete/releasable is
the product" — written from the macOS lead frontend. Windows was originally filed inside **Phase 4
("Top-tier"), after the 1.0 milestone** (`rm §5.4`), listed alongside the AI layer, mobile companion,
and depth work. That placement read as "Windows comes *after* Phase 3 ships."

Two things make that placement wrong now:

1. **The architecture already decouples it.** ~80% of behavior lives in the shared Rust core; a second
   frontend is native view + binding over the same core + FFI (both done). Nothing about a Windows
   shell depends on reaching Phase 4 — it depends only on the core, which stands (`spec §16`).
2. **It's actually starting now.** The public `README.md` states "Windows is next," and work is
   beginning against `parity.md`'s Win column while macOS MVP1 completes.

Leaving the strategy doc saying "Windows = Phase 4" would misdescribe the real order of work and
invite a future agent to defer Windows behind AI/sync features it doesn't depend on.

## Decision

**Platform expansion is an orthogonal track to the phase plan.** Windows is a **parallel frontend
track**, gated only on the shared core (done), started now and built against `parity.md`'s Win column
concurrently with the macOS-lead Phase 2/3 feature work — *not* sequenced behind Phase 4. The phase
plan continues to describe the public-product bar from the macOS lead; a concurrent Windows build
catches up to parity beneath it, and the two converge.

Captured in `rm §5` (the callout + the §5.4 Windows bullet) and `spec §8,§16`; `parity.md`'s Win
column is the build-spec.

## Alternatives

- **Leave Windows in Phase 4, sequential** — rejected because it misrepresents the dependency graph
  (Windows needs the core, not Phase 3's feature depth) and would defer a second frontend behind
  unrelated AI/sync work, contradicting both the README and the work actually underway.
- **Create a dedicated `docs/windows.md` execution doc (mvp2-style M1–M5)** — considered and declined
  for now (user's call, 2026-07-13): the existing docs carry it — `parity.md`'s Win column is the
  checklist, `spec §8` the structural spec, `spec §16` the sequencing. Revisit if the Windows plan
  outgrows being distributed across those files.
- **Re-number the phases to insert a "Windows" phase** — rejected: phases are the *product bar*, not a
  frontend schedule; forcing Windows into the linear phase sequence recreates the same sequencing
  error in a different place. Orthogonal track is the truer model.

## Consequences

- The roadmap now reads as "macOS-lead product bar (Phase 2/3) with a concurrent Windows build catching
  up to parity" — an honest description of the real order of work.
- **Parallel-work discipline matters more.** A Windows frontend advancing alongside macOS feature work
  means the shared core + FFI are edited from two frontend directions; the `code §4` parallel-edit
  hotspots (the FFI surface, the `StructureChanged` snapshot boundary) are now load-bearing for
  cross-frontend, not just cross-agent, safety.
- **No new tracking doc.** The Windows plan stays distributed across `parity.md` (checklist), `spec §8`
  (structure), and `spec §16` (sequencing). If that proves too thin once Windows work is underway, a
  dedicated execution doc is the escape hatch (see Alternatives).
- Does not change any Phase 2/3 *content* — only how Windows relates to the timeline. The AI layer,
  sync, and mobile remain Phase 4; Windows simply leaves that bucket.
