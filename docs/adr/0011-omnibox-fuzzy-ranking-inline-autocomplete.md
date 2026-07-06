# ADR-0011: Command-bar autocomplete — purpose-built fuzzy ranking + inline autocomplete

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

The command bar's history autocomplete matched with a SQL `LIKE '%q%'` substring and ranked by raw
`visit_count, last_visit`. That misses everything a modern omnibox does: `gh` won't surface
`github.com` (not a contiguous substring), a match on the domain start ranks no higher than one
buried in a query string, and there is no inline autocomplete — the signature behavior where typing
`git` fills the field with `github.com` and selects the completion so Return goes straight there.

Two real decisions: **where** the matching/ranking lives, and **how** it's built.

- *Where*: the project's core rule is that browser logic is Rust and frontend-agnostic (a second
  frontend must not reimplement it). Matching + ranking + highlight computation therefore belong in
  `crates/core`, behind the existing suggestion FFI; the native layer stays view + binding.
- *How*: a generic fuzzy crate (e.g. `nucleo-matcher`) vs. a small purpose-built scorer. And how far
  to take it — dropdown ranking only, or inline autocomplete too.

## Decision

A **purpose-built, dependency-free fuzzy scorer** in the core (`crates/core/src/fuzzy.rs`) plus a
**class-based ranking policy** in `history::query`, and **inline autocomplete** in the command bar.

- **Scorer** — a Smith-Waterman-style DP (the two-matrix "match-ended vs best-so-far" formulation, so
  the consecutive-run bonus is applied correctly) over Unicode scalars. It rewards boundary-aligned
  matches (string start, after a separator, camel hump), adjacent runs, and the string start; it
  returns the score **and the matched character indices** for highlighting.
- **Ranking** — each candidate is bucketed into a *class* by where the query matched: host **prefix**
  > host fuzzy > URL > title-only. The class base dominates the score (bases are spaced wider than
  any reachable detail + frecency bonus), so match quality beats frecency; within a class, the fuzzy
  detail plus a **log-scaled frecency bonus** (visit count × recency-bucket weight) order the rows.
  Candidates are the `SCAN_LIMIT` (2000) most-frecent rows, scored in memory.
- **Inline autocomplete** — the top-ranked host-prefix match is the *default match*. The core exposes
  each row's normalized `displayUrl`; the controller offers a completion when the typed text is a
  clean prefix of the default match's URL. The command bar fills the field and selects the appended
  tail, tracking the user's typed text separately from the field text and suppressing the completion
  on deletion (so it never fights a backspace).
- **Two-tier providers** — local providers (typed-URL, history) run with no debounce so results and
  the inline completion are instant; the remote search-suggest provider is debounced and merges below.

## Alternatives

- **Keep SQL `LIKE` + frecency** — rejected: no subsequence/acronym matching, no match-quality
  ranking, no matched ranges to highlight, and no basis for a default match. It's the gap being fixed.
- **`nucleo-matcher` (or similar) crate** — rejected: adds a dependency against the project's "no dep
  you don't need" ethos, and it's a *generic* subsequence scorer. URL ranking is domain-specific
  (host prefix must dominate a path hit; www/scheme normalization; host-vs-path-vs-title classes) —
  that policy lives outside any generic matcher anyway, so the crate would only replace the ~120-line
  DP while we still hand-write the ranking. Not worth the dependency. (Revisit if we later want the
  same matcher across many surfaces.)
- **Blended score (match × frecency) instead of hard classes** — rejected for now: harder to reason
  about and to keep predictable (a hugely-visited page could outrank a clean host-prefix match). Hard
  class tiers make the default match — and therefore inline autocomplete — deterministic. The blend
  is available later if tiers feel too rigid.
- **Dropdown ranking only, no inline autocomplete** — rejected: inline autocomplete is the defining
  omnibox behavior the request asked for. It costs more (typed-vs-field text tracking, a two-tier
  controller), but that complexity is contained in the command bar + controller.
- **Fuzzy-score the whole history table every keystroke** — rejected: unbounded cost as history
  grows. Scoring the 2000 most-frecent rows covers the pages actually worth completing to and keeps
  per-keystroke cost flat. (Documented constant, not a silent cap.)

## Consequences

- Matching/ranking/highlighting are core-tested (`fuzzy` + `history` unit tests) and reused verbatim
  by any future frontend; the FFI suggestion JSON gains `displayUrl`, `titleMatches`, `urlMatches`.
- The `SCAN_LIMIT` bound means a genuinely rarely-visited page whose only relevance is an exact match
  can fall outside the scored set. Acceptable for a personal browser; raise the bound or add a
  targeted SQL prefilter if it ever bites.
- Typo tolerance is limited to what subsequence matching gives (dropped/extra characters), **not**
  substitutions or transpositions (`githbu` won't match `github`). Matches mainstream URL bars, which
  don't heavily typo-correct; edit-distance is a later option if wanted.
- Inline autocomplete's SwiftUI `TextField` selection UX (filling + selecting the tail, suppressing on
  delete) is state-heavy and can't be unit-tested here; it needs an **interactive verification pass**
  (blocked behind the same running-app/focus-routing work as P0-1 / M2).
- The frecency weights and class bases are tuning constants; they're documented and test-pinned, and
  can be adjusted without structural change.
