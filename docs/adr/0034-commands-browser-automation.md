# ADR-0034: Commands — user-defined browser automation, unified in the command bar

- **Status:** Accepted
- **Date:** 2026-07-08

## Context

The browser's hackability story has two distinct layers that must not be conflated:

- **Tweaks** (ADR-0002, spec §20) — per-site *page* customization (Snip / Restyle / Script), injected at
  document-start into the web view.
- **CDP surface** (P0-4, spec §13) — loopback-bound external automation for dev tools and agents.

Neither covers *browser-chrome automation*: switch to a tab by URL pattern, spawn a terminal with the
current repo, copy the active URL, run `gh repo clone` from a GitHub page. Glide ([introducing-glide](https://blog.craigie.dev/introducing-glide/)) demonstrates the demand — a TypeScript config with tab/process/path APIs and keyboard mappings that extensions cannot safely provide.

The command bar (⌘T, ADR-0008/0011) already unifies URL/search/tab suggestions via `SuggestionProvider`;
`SuggestionProviders.swift` notes that adding a source is "just another value in the provider list." The
roadmap's Phase 3 "command palette" (⌘K) was listed separately with no stated relationship to user scripts.

Terminology: **Script** is reserved for the JS tier of Tweaks (style guide §13). Browser-level automation
needs its own name.

## Decision

Build **Commands** — user-defined browser automation scripts — as a core-owned subsystem distinct from
Tweaks and the external CDP port.

**What a Command is.** A saved TypeScript module that runs in a sandboxed runtime with a typed
`CommandContext` API backed by the Rust core (`tabs`, `spaces`, `navigate`, `process.exec`, `path`,
`clipboard`, `notify`, …). One file per command under `root_cache_dir()/commands/`, with a metadata
header (`@name`, `@description`, `@keywords`, optional `@when` URL guard). Portable, sync-ready later.

**Invocation surfaces (two, complementary):**

| Surface | Shortcut | Role |
|---|---|---|
| **Command bar** | `⌘T` | **Primary, always.** Unified ranking: URLs, open tabs, history, search, *and* Commands in one list. New-tab navigation stays first-class; Commands appear when they match the query. Empty query may surface recent/favorite Commands. |
| **Command palette** | `⌘K` | **Secondary, Phase 3.** A do-anything palette biased toward *actions* — built-in browser actions plus the same Command registry. No URL/search navigation; pure action dispatch. Shares the core registry and `CommandsProvider` ranking logic. |

`⌘T` is never demoted or replaced. The palette is an additional entry point, not a migration.

**Settings.** A **Commands** section in the native settings panel (ADR-0010): list, enable/disable,
edit (native monospace TS editor), duplicate, delete, import/export, inline test runner, and a built-in
cookbook (e.g. "go to tab by URL", "clone GitHub repo", "copy URL as markdown link"). Mirrors the
Tweaks manager pattern (style guide §14) but for chrome automation.

**Vibe-coding UX (Phase 3).** TypeScript for human + external-agent authoring parity with Glide; generated
API type stubs for editor autocomplete; inline test against current context (Glide's `:repl` equivalent);
built-in cookbook.

**Agentic create/edit (Phase 4, future AI).** In-app agentic authoring — describe a workflow in natural
language (or iterate in chat) and the agent **creates or edits a Command** in the settings manager, shows the
generated TypeScript, and requires the same enable consent as hand-written scripts. Same object store as
manual edits (`command_upsert`); not autonomous execution. Pairs with the agent layer (spec §19) and the
Tweaks agent surface (§20) — Commands are the chrome-automation counterpart. Deferred past P3-16; not
required for Commands v1.

**Safety.** User-owned config (Glide model — no extension sandbox). Still gated like Tweaks Script:
code shown before enable, re-prompt on change, global kill switch. `process.exec` requires an explicit
permission tier (aligned with the agent permission model, spec §19). Agent-authored Commands are the
same object as hand-written ones.

**Core module:** `commands` — registry, file watcher, runtime host, permission checks, FFI
(`commands_query`, `command_run`, `command_upsert`, …). Not the external CDP `automation` wrapper.

**Phased delivery** (backlog P3-14…P3-17): (1) registry + built-ins + command-bar provider; (2) user TS
scripts + settings manager; (3) vibe-code UX (stubs, test runner, cookbook); (4) optional per-Command
keyboard bindings (pairs with Phase 3 keyboard-first nav).

## Alternatives

- **Extend Tweaks with a fourth tier** — rejected: page injection and chrome automation share no
  mechanism; conflating them would leak page-scoping into tab/process APIs and confuse the vocabulary.
- **Shell-only / CDP-only** — rejected: poor vibe-coding UX; external CDP port is consent-gated and
  dev-oriented, not a daily-driver customization surface.
- **Single `glide.ts` config file** — rejected: one-file-per-command is easier to generate, share, enable
  independently, and diff in settings.
- **Commands only in ⌘K, keep ⌘T navigation-only** — rejected (user 2026-07-08): `⌘T` is the main
  browser entry point; Commands belong in the unified bar. `⌘K` is additive for action-first dispatch.
- **Rust/WASM plugins** — rejected for v1: high authoring friction, poor agent training-data coverage
  vs TypeScript.

## Consequences

- Positive: Glide-class hackability without a Firefox fork; reuses tab model, suggestion stack, settings
  patterns; clear separation from Tweaks and CDP.
- Positive: `⌘K` palette has a concrete extensibility primitive on day one (the Command registry).
- Negative: embedded TS runtime is new dependency surface (evaluate Deno core / QuickJS + transpile).
- Negative: two entry points to keep ranking consistent — shared `CommandsProvider` + core query API
  prevents divergence.
- Follow-up: ADR for runtime choice at implementation time; keyboard bindings row (P3-17) depends on
  Phase 3 keyboard-first nav.