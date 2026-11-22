# ADR-0078: One inline completion, chosen by how much the site is used

- **Status:** Accepted
- **Date:** 2026-11-22

## Context

ADR-0071 lets any page the user typed before complete the field inline. When two typed sites share
the input's start ("you": youtube.com and yoursmileis.art), each became a completion row at the same
relevance. Frecency counted at most 10 visits, so 960 YouTube visits scored like 9 visits to a saved
store page, and the tie broke on the alphabetical key. The rarely used site took Enter. Accepting it
also stored a learned pick ("you" -> yoursmileis.art), which made the wrong answer stick.

Arc-imported typed counts make this common: Arc counts opening a saved tab as typed, so many saved
pages look typed.

## Decision

- Every page that may complete inline is only a candidate. Exactly one candidate completes and takes
  the inline relevance; the rest stay ordinary history rows.
- The winner is the fill (host, or path prefix) with the most use: the sum over every page under it
  of `(visits + 2 x typed) x recency weight`, on a log scale, plus the learned-pick strength for this
  input. Ties go to the more typed site, then the shorter fill. This follows Firefox's origin
  autofill (one origin, highest frecency, summed over the host) and Chromium's HistoryURLProvider
  (one inline match; more typed wins).
- Frecency is no longer capped at 10 visits, here or in history row ranking.
- Menu command rows need every query word to start a word of the command. They sit right under the
  default, so a mid-word fuzzy hit ("you" in "Layout") must not reach that slot.

## Alternatives

- **Firefox's adaptive-history-first order** (a learned pick always beats frecency) - rejected: the
  learned data already holds picks made by accepting a wrong default. Letting it win keeps the bug.
  As a bonus it still decides close cases.
- **Stop learning from accepted defaults** - rejected: Chromium learns from them too, and once the
  default is right this reinforces the right answer. The cause was the tie, not the learning.
- **Ignore imported typed counts** - rejected: they are the best signal on day one for most sites.
  Summed use already puts a saved-but-unvisited page below a daily site.
- **Clean the polluted learned rows** - rejected: they decay on their own (2.5% per day) and the
  new score outweighs them.

## Consequences

- The inline site is stable and matches what the user visits most. A less used site completes once
  the input is specific enough ("yours").
- The winner scan reads the history candidates once per candidate fill. Candidates per keystroke
  are few, so this is cheap.
- Guards: `the_most_used_site_wins_inline_when_several_share_the_prefix` (real numbers from the
  report) and the `commands-need-word-starts` case in `just newtab-selftest`.
