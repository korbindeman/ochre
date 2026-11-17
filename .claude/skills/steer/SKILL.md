---
name: steer
description: >-
  Roadmap steering harness for this browser project. Use when the user asks "what's next?" /
  "what should I work on" (no specific task named); when discussing future vision, long-horizon
  direction, or reopening a strategic fork that should be captured and turned into work; or when
  adding a feature / filing a bug that should be tracked. Reads docs/roadmap.md (strategy) and
  docs/backlog.md (execution) and routes to one of three modes - scope the next item, evolve the
  vision, or capture concrete work - keeping the backlog in sync as work lands.
---

# Steer - the roadmap harness

This project self-steers through these documents plus this skill:

- **`docs/roadmap.md`** - *strategy*. Thesis, positioning, the phased plan (§5), the open forks (§10).
  Slow-changing. Answers *why* and *in what order*.
- **`docs/backlog.md`** - *execution*. A rolling, status-tracked queue of concrete scoped items.
  Answers *what's the next thing*.
- **`docs/parity.md`** - *coverage*. The capability × frontend status grid (ADR-0007). **Not a routing
  input** - it's a write-target, kept in sync when a frontend-facing capability lands (see "Keeping the
  trackers honest"). Consult it when scoping frontend work to see where a platform stands.
- **This skill** - the routing + scoping procedure that ties them together.

**Always read the roadmap + backlog at the start of an invocation**, then route to one of three modes.

## Detect the mode

- **Mode 3 - "what's next?"** - user asks what to work on / for the next item, no specific task named.
  → Scope the next item.
- **Mode 1 - concrete work** - user names a specific feature or bug ("add X", "fix Y").
  → Capture, then do.
- **Mode 2 - vision** - user talks about direction, aspiration, "where this should go", or reopens a fork.
  → Capture to the roadmap, then decompose into work.

When genuinely ambiguous, ask one clarifying question rather than guessing.

## Mode 3 - "what's next?" (propose + scope, then stop)

**Contract: propose the next item, scope it into an actionable brief, then stop for the user's go-ahead.**
Do **not** start implementing, and do **not** flip the item to `wip`, until the user greenlights.

1. **Reconcile.** Skim recent `git log` + the working tree against `backlog.md`. Correct any status that's
   gone stale (e.g. marked `next` but already landed). A quick sanity pass, not an audit.
2. **Candidate pool** = items with status `next` (no unmet deps). If the pool is empty, the top `blocked`
   item's blocker becomes the candidate - often a pending fork or an external / user action; surface *that*
   as the next action.
3. **Rank**, in order: (a) lower phase first; (b) gating risk / unblocks the most downstream items;
   (c) explicit priority notes; (d) smaller estimate as a momentum tiebreaker.
4. **Pick the top item** and write the brief:
   - **Item** - id + title.
   - **Why now** - where it sits, what it unblocks, why it beat the others.
   - **Goal** - the outcome, one line.
   - **Approach** - the sketch: key files / modules, the shape of the change.
   - **Exit criteria** - verifiable done-conditions (tie to the phase exit / mvp DoD where relevant).
   - **Risks / unknowns.**
   - **Est.** - tokens.
5. **Stop.** Hand the brief back. On go-ahead: flip the item to `wip` in `backlog.md`, then begin.
   If the user redirects, pick again.

If the honest answer is *"the real next step is a decision, not code"* (an unresolved fork gates everything
queued), say so and switch to Mode 2 on that fork.

## Mode 1 - concrete work (capture + do)

1. **File it** in `backlog.md` as a `next` item - right phase, next id, note deps / est. Even a quick fix
   gets a one-line row; the backlog is the record of what was done.
2. **Do the work** - normal implementation, obeying `CLAUDE.md`.
3. **Close the loop** - mark the item `done` (with date). If the work landed or changed a capability
   tracked in `parity.md`, flip its cell for that frontend (see "Keeping the trackers honest"). If the
   work surfaced new work, file those as `next` items rather than dropping them (`CLAUDE.md`: no
   deferred TODOs - track them here).

## Mode 2 - vision (capture to roadmap, then decompose)

1. **Discuss** and sharpen the aspiration with the user.
2. **Record it in `roadmap.md`** at the right altitude - extend a phase's scope, revise a fork, or add a
   consideration. Keep the roadmap's voice (for forks: tension → options → recommendation). If a fork is
   *resolved*, also write an **ADR** (`docs/adr/`, per its README) and flip that fork's row in the
   `backlog.md` "Decisions pending" table.
3. **Decompose** the new direction into concrete `backlog.md` items (`next` or `later` by phase). This is
   the step that turns vision into pickable work - don't skip it.
4. **Don't implement** in this mode unless the user then asks. Mode 2 produces plans + items, not code.

## Keeping the trackers honest (every mode)

- Mark items `done` the moment they land; keep statuses truthful - a stale backlog makes "what's next?" wrong.
- **Sync `parity.md` when a frontend-facing capability lands or changes** - flip its cell for that frontend
  (`✅ / 🚧 / ⬜`). This is part of "done," exactly like marking the backlog item. A cross-frontend gap on
  the critical path becomes a `next` backlog row: the ledger is the coverage map, the backlog is the queue.
  Update status only - behavior stays defined in `style`/`spec`, linked per row (one source of truth).
- Respect the **rolling window** - itemize only the current + adjacent phase; leave far-future work at phase
  granularity in the roadmap until it nears.
- New work discovered mid-task → a `next` row, never a silent TODO.
- One source of truth - rationale lives in the roadmap / ADRs; the backlog holds status + pointers.
