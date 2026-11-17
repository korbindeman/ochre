# ADR-0001: Roadmap steering harness - strategy / execution split + `steer` skill

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

The project needed a repeatable way for the user to drop in and always know what to do next - in
particular to ask **"what's next?"** and have the agent decide and scope the next piece of work. Three
recurring interaction shapes were identified: (1) concrete work ("add X / fix Y"), (2) long-horizon vision
that should be captured and turned into work, and (3) "what's next?" that scopes the next item autonomously.

`docs/roadmap.md` already existed and is strong, but it is a **strategy** document - thesis, positioning,
phases, and the §10 forks. It has no status, no backlog, no dependency ordering, so it cannot answer "what
is the single next thing to pick up." Making one prose document serve as both strategy and work-tracker is
the classic way roadmaps go stale.

## Decision

A two-layer split plus a harness skill:

- **Strategy layer** - `docs/roadmap.md` (unchanged in role): vision, phases, forks. Slow-changing.
- **Execution layer** - a new `docs/backlog.md`: a single markdown file holding a rolling, status-tracked
  queue of concrete scoped items (`next` / `wip` / `blocked` / `done` / `later`), with stable ids,
  dependencies, token estimates, and refs. A **rolling window** - only the current + adjacent phase are
  itemized; later phases stay at phase granularity in the roadmap until pulled in.
- **Harness** - a committed project skill, `.claude/skills/steer/SKILL.md`, that reads both docs and routes
  to one of the three modes. For "what's next?" the contract is **propose + scope, then stop**: pick the top
  `next` item, write an actionable brief, and wait for the user's go-ahead before writing code.

## Alternatives

- **Extend `roadmap.md` to be the work-tracker too** - rejected: conflates slow-changing strategy with
  fast-changing status; the churn would erode the strategy doc and the two altitudes fight in one file.
- **One file per work item (`docs/work/NNNN-*.md`)** - rejected for now: more scalable and richer per-item
  history, but heavier ceremony per add; premature for a solo, pre-public, single-repo project. Revisit if
  item volume outgrows a single scannable file.
- **GitHub Issues as the backlog** - rejected for now: the project isn't public (roadmap puts GitHub at
  Phase 2) and it makes "what's next?" depend on a network round-trip. Graduate to it when public.
- **Convention doc only, no skill** - rejected: not directly invocable and easy to drift from; the user
  explicitly wanted an invocable harness.
- **"What's next?" auto-starts the work** - rejected: removes the user's checkpoint before code lands. The
  chosen contract stops after scoping.

## Consequences

- The agent must keep `backlog.md` honest - mark items `done` as they land, file discovered work as `next`
  rows (no silent TODOs), and reconcile stale statuses on each "what's next?" run. A stale backlog makes the
  harness wrong.
- One-source-of-truth discipline: rationale lives in the roadmap / ADRs; the backlog holds only status +
  pointers. Resolving a fork updates the roadmap **and** writes an ADR **and** flips the backlog's
  "Decisions pending" row.
- Two new near-term seams to migrate later, by design: single-file → per-item files, and local file →
  GitHub Issues, both when scale/publicness warrants (thresholds noted above).
- The backlog was seeded from roadmap §5 + `mvp1.md`; initial statuses are a first pass to be
  reconciled against the working tree on first use.
