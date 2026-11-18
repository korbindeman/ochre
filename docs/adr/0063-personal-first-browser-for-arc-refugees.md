# ADR-0063: Ochre is a personal browser first, for Korbin and friends leaving Arc

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

The roadmap (`rm §1`, `rm §5`) was written in July 2026 around a public open-source launch: a public
beta (Phase 2), a credible public 1.0 (Phase 3), governance and funding (`rm §7`), and a "1.0 bar"
(`rm §6.5`). Since then Arc is shutting down. Korbin and a small group of friends, all on macOS, will
move to Ochre. What they need is what Ochre already has: Arc import, Spaces, and the tab model
(Favorites, Pinned, Open).

## Decision

**Ochre is Korbin's personal browser first, shared with friends leaving Arc.** A public launch is
parked, not cancelled.

- Priority order: anything that stops an Arc user from switching or staying (data loss, crashes,
  import gaps, tab and Space behavior) ranks above public-launch work.
- Friends get Developer ID signed, notarized builds from GitHub Releases, kept current by the
  existing auto-update (ADR-0052, `alpha-release.yml`). No hand-delivered builds.
- macOS only for the friend group. No friend needs Windows.
- Public-launch work (`rm §6.5` 1.0 bar, `rm §7` funding and governance, positioning against Zen)
  drops to `later`. Product thesis and interaction model are unchanged.

## Alternatives

- **Keep the public-beta plan as the main line** - rejected: the real users are a known small group
  with known needs, and a public bar (governance, funding, broad web-platform gaps) delays what they
  need without helping them.
- **Hand-delivered builds for friends** - rejected: every fix would need a manual reinstall; signing,
  notarization, and auto-update already exist.

## Consequences

- `rm §1` and `rm §5` read through this lens; the phase table keeps its shape but Phase 2 is the
  friends' daily driver, not a public beta.
- Reliability of core state (`tabs.rs`, `session.rs`, import) becomes the top risk; see ADR-0065.
- Revisit when the friend group is stable on Ochre, or if a public release becomes the goal again.
