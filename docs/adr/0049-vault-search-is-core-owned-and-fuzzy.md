# ADR-0049: Vault search is core-owned, fuzzy, and shared by every surface

- **Status:** Accepted
- **Date:** 2026-07-16

## Context

The URL-bar vault popover (P2-34) lists only `vault_lookup(host)` matches — logins whose URIs match
the active tab. A login filed under a different host is unreachable from the page you need it on:
a shared SSO account, a renamed site, an entry saved under a bare app name. The only escape was to
open the Passwords page. This was the "we can search for others" half of the request that produced
ADR-0048; it was split out so the ordering fix could ship alone (P2-42).

Adding search surfaced a pre-existing problem. The Passwords page **already** searched — in Swift
(`VaultView.swift`, `VaultModel.filtered`): a lowercased `contains` over name, username, and host.
That is browser logic living in a frontend, which the project's core-first rule forbids precisely
because a second frontend (Windows) would have to reimplement it and would drift. Building a second,
core-side search for the popover while leaving that in place would have meant **two divergent search
semantics over the same vault** — the worst of both.

The core already has a search ranker: `fuzzy.rs`, the omnibox scorer, whose own module doc sets the
split — the scorer answers "how well does this query match this one string", and the ranking
*policy* lives with the domain (`history.rs` for history).

## Decision

**One search entry point in the core; every surface calls it.**

1. **`vault::search::search_logins(vault, query, usage)`** — the policy. A login matches if the query
   fuzzy-matches its **name**, **username**, or any **URI host**, and scores as its best field.
   Ranking is score-first, then last-used, then vault order — the same tie-break chain as
   `lookup` (ADR-0048), so recency is a tie-break and never outranks a better field match.
2. **Ranking reuses `fuzzy.rs`**, so searching the vault feels like searching history or the URL bar
   (subsequence matching, biased toward word-boundary and consecutive hits) rather than inventing a
   third matching semantic in the same product.
3. **An empty query lists the whole vault**, so the filtered and unfiltered list states are one call
   and cannot disagree.
4. **`browser_vault_search(query)`** exposes it; the popover's new search field **and** the Passwords
   page's list both consume it. `VaultModel.filtered` is now core-ranked, and the Swift filter is
   deleted rather than left beside its replacement.

The **un-searched popover list stays site-scoped** (`lookup`) — search widens scope only when the
user types. Scope and ranking stay separate concerns.

## Alternatives

- **Filter in the frontend** (extend the existing Swift `contains`) — rejected: it is the wart this
  ADR removes. Every frontend would reimplement the semantics, and they would drift silently. The
  core-first rule exists for exactly this.
- **A core search for the popover, leaving the Passwords page's Swift filter** — rejected: two search
  implementations over one vault, guaranteed to diverge; the same query would rank differently in two
  places in the same app.
- **Substring `contains` in the core** (port the Swift semantics down) — rejected: it would make the
  vault the only surface in the browser that does not fuzzy-match, and `fuzzy.rs` already exists and
  is tuned. Substring is also strictly weaker (`gh` finds nothing in `GitHub`).
- **Search the notes / TOTP / password fields too** — rejected: notes hold arbitrary secrets, and
  matching a *password* would let a shoulder-surfer confirm a guess by watching results filter. Name,
  username, and host are the identifying fields, which is what the managers match on.
- **Autofocus the search field on open** — rejected: the primary action is picking from this site's
  list; stealing focus to a search box would make the common case slower. Bitwarden's popup does not
  autofocus either.
- **Rank site-matching logins above others while searching** — deferred, not rejected: plausible, but
  it conflates scope with ranking, and no evidence yet that a site-match tiebreak beats a plain score.
  Revisit if search results feel wrong on a page with matches.

## Consequences

- The popover becomes a complete credential chooser: this site's logins by default, the whole vault
  on demand, with Fill/Copy working on any result (`fill_active_tab` takes an id, not a host).
- The Passwords page's search changes behavior: fuzzy instead of substring, and now recency-tiebroken.
  Strictly more forgiving — every substring hit is also a subsequence hit — so nothing findable before
  becomes unfindable.
- Windows inherits search for free; its Passwords page is view + binding, as intended.
- One more core call per keystroke (an in-memory scan of an unlocked vault, no I/O beyond a usage
  snapshot). Fine at vault scale; if a huge vault ever makes it jitter, cache the snapshot rather
  than reintroducing frontend filtering.
- `search` returns `LoginView` — metadata only. Passwords still cross solely via `password_for` on an
  explicit gesture (ADR-0017), asserted at the FFI boundary by `crates/ffi/tests/vault_search.rs`.
