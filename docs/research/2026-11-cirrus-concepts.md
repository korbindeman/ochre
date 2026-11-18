# Concepts from cirrus worth reusing in Ochre (2026-11-18)

Read from `korbindeman/cirrus` (private). Paths are relative to that repo.

1. **Law register** (`.laws/laws.toml`): each rule has `id`, `what`, `how`, `paths`/`skip`, a `match`
   regex, and `finds`/`passes` examples that test the rule itself. Rules marked `may_rise` keep per-file
   counts (`.laws/*.counts`) that only go down. Ochre: generalize `emdash-check` / `privacy-check` into
   one rules file + a small `scripts/laws.py`; post-mortems become rules. (P1-48)
2. **Lint registers** (`lib/rust/reinforce.toml`, `lib/rust/explore.toml`): reinforce denies `unwrap_used`,
   `expect_used`, `indexing_slicing`, `panic`, `exit`, `as_conversions`, `await_holding_lock`,
   allow-without-reason. Ochre has no `[lints]` table; `crates/ffi` and the crash-sensitive core modules
   matter most because a panic across the C ABI aborts the app. Baseline counts ratchet down. (P1-49)
3. **Statecharts as data** (`vendor/vitae/crates/statechart`, ~1,100 lines, deps serde/serde_json/thiserror):
   JSON SCXML subset, `validate`, a pure `step` returning effects, `paths::explore` walking every reachable
   configuration. Tests read like "exactly one surface is open in every reachable configuration". Ochre:
   the UI surface state machine (P1-46). Same pattern, local implementation, because vitae is deferred.
4. **Computer-use protocol** (`ai/computer/`): one tool catalog, `look` (numbered controls + text, then
   only what changed), `do` (step grammar with label matching), adapter conformance cases, MCP stdio
   server, a DOM program (`browser/page.js`) that walks shadow roots and masks secret fields. Safety:
   agents work in their own space by default, touch the user's screen only visibly and when allowed.
   Ochre: shapes the agent endpoint (P2-51/P2-52).
5. **Interaction tests that leave evidence** (`docs/reference/interaction-tests.md`): a failed wait saves
   a screenshot and logs to an evidence folder; bugs become tests named as invariants; every UI feature
   lands with a test for its main path. Ochre: evidence on selftest failure (P1-50).
6. **Path-scoped guards** (`land.toml [[precheck.guards]]`): each guard lists the paths it reads; only
   affected guards run. Ochre: map selftests to paths and run the affected ones (P1-50).
7. **Code rules** (`.agents/skills/nomos-code/SKILL.md`): turn "do not remove" comments into types/tests/
   lints; state changes converge when rerun; old records stay readable; every bug fix has a test that
   fails before; every state of a state machine has a named test; enums instead of synced booleans.
8. **Flaky and baseline registers** (`.aether/flaky.toml`, `tests/userland-baseline.toml`): a diagnosed
   flaky test is advisory with a reason and ticket; baselines only go down.

Also: per-directory AGENTS.md read outermost first; edit-time hooks that refuse banned lines; the
navigation machine (`vendor/vitae/crates/navigation`) refuses impossible steps with a typed `Refused`,
a pattern for `tabs.rs` mutators; no-polling rule for selftests.
