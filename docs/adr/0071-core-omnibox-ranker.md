# ADR-0071: The omnibox ranks in the core, on Chromium's relevance scale, and learns from picks

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

The command bar and the address editor ranked suggestions in Swift with hand-tuned score tiers. Return committed the field text unless the user had arrowed to a row, so a top row reading "Switch to Tab: GitHub" did not run on Return. The top row could change as remote suggestions arrived. Inline completion filled any host prefix from history or a hardcoded top-sites list, and nothing learned from what the user picked. Korbin, 2026-11-18: "a modern experience where we get what we want generally".

Research against primary sources (Chromium `components/omnibox/browser`, Firefox urlbar docs) gave concrete rules.

## Decision

- **`crates/core/src/omnibox.rs` ranks.** `rank` is pure and `query` gathers its inputs: history pages, learned picks, and sidebar tabs. The host fetches remote search suggestions and adds its own command rows under the default row.
- **One relevance scale (Chromium):**
  - inline completion: 1410+
  - search for what you typed: 1300
  - URL you typed: 1203
  - other history: at most 1399, by match class (URL prefix, host, URL, title), fuzzy detail, frecency and learned picks
  - search for a URL-like input: 850
  - remote suggestions: 800 and down
  - Open and saved tabs get a small lift.
- **Default row:** only a row the input leads to may be the default: the typed URL, the typed search, or an inline completion. It is shown first and preselected, and Return runs it.
  - When remote suggestions arrive, or the input is 4 or more characters, the previous default stays if it still qualifies (Chromium's rule).
- **Inline completion:**
  - Only for a single token, at the start of the host (after the scheme or `www.`), never after a deletion or paste.
  - Completing a host needs it to have been typed once; completing a deeper URL, to the next slash, needs it typed twice. Picking it twice for the same input also qualifies.
  - Hosts are completed before paths.
- **Learning (Firefox adaptive history):**
  - Running a row stores (input, URL). The count decays by 0.975 per day, then `x 0.9 + 1`.
  - Stored inputs that start with the typed text rank that URL higher, and an exact match counts double. Entries unused for 90 days are pruned.
  - Taking the default URL row also increments `typed_count`, which gates inline completion.
- **Tabs:** a row whose URL is open in a sidebar tab switches to that tab (Arc). Private windows never switch to a normal-window tab.
- **Dedup and layout:**
  - Duplicates are matched on the URL without scheme, `www.`, fragment, trailing `/` or empty `?`.
  - At most 8 rows, in this order: the default row, then up to 5 pages and tabs, then up to 3 searches. Spare room goes to the other group.
- **Empty field:** the most frecent pages, open ones as tab switches. Return does nothing.
- **Removed:** the Swift providers for history, open tabs, typed URL, search fallback and hardcoded top sites; `browser_query_suggestions`; the history crate's own ranking.

## Alternatives

- **Keep ranking in Swift and fix Return.** Rejected. A second frontend would reimplement it, and the rules need table tests.
- **Search rows above pages (Chromium and Firefox desktop default).** Rejected for now. Arc users reach for tabs and pages; revisit if search-heavy use shows otherwise.
- **Default the strongest learned pick even when the input does not spell its URL** ("email" leading to Gmail). Rejected. Chromium allows a default only if the input leads there; that row sits second instead.
- **Interaction-based frecency (Firefox 2024).** Deferred. It needs dwell and keypress signals the core does not record yet.

## Consequences

- `history` gains a `typed_count` column (migrated on open) and an `input_history` table.
- **Tests:** 16 table tests cover default choice, inline gates, deletion and paste, learning, tabs, remote stability, dedup and caps, the empty field, non-ASCII safety, and a property that the default is always something the input leads to. `just newtab-selftest` checks that the top row is preselected and runs on Return.
- Rows commit through the surface machine (ADR-0068), so destinations stay correct.
