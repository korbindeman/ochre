# ADR-0048: The autofill list orders by last-used, within match tier

- **Status:** Accepted
- **Date:** 2026-07-16

## Context

The URL-bar vault popover (P2-34, ADR-0029) and the in-page picker (P2-16) both list the logins
matching the active tab's host, ranked by `vault::lookup::matching_logins`. That ranking had exactly
one signal: **match quality** — an exact-host match (`MatchQuality::Host`) outranks a
registrable-domain match (`Domain`). Within a tier the sort was *stable*, so entries kept the vault's
own sync order.

That order never changes. A user with several accounts on one site got the same fixed list forever,
with the account they actually sign in with sitting wherever Bitwarden's `/sync` happened to place
it — reported as "the password list is always in the same order" (user report 2026-07-16).

Bitwarden's cipher model gives us nothing to sort by: `revisionDate` is the *edit* date, not a use
date, and the server has no last-used-per-cipher concept. Bitwarden's own extension tracks recency
client-side for the same reason. So the signal has to be ours, local, and new.

## Decision

**Track last-used locally and use it as the tie-break *within* a match tier.**

1. **`vault::usage`** — a `login id → unix seconds` map at `root_cache_dir()/vault-usage.json`.
   Plaintext, atomic writes, lazily loaded, never synced.
2. **`matching_logins(vault, host, usage)`** takes the map as a **parameter**. Sort key is
   `(MatchQuality, Reverse(last_used))`: match strength still ranks first, recency orders within a
   tier, and never-used entries (implicit `0`) sink to the bottom of their tier in vault order. An
   empty map reproduces the old behavior exactly.
3. **`service::note_used(id)`** is the single choke point that records a use — and the single place
   the private-window rule is enforced.
4. **Only explicit use gestures record**: filling the active tab (`embed::vault_fill_active_tab`),
   picking an entry in the in-page picker (`autofill.rs`, on a real password hit), and copying the
   username/password from the popover (host-side, via `browser_vault_note_used`).

Recency is deliberately **not** recorded by `password_for` / `login_detail` themselves, even though
every gesture flows through them. Those are also the Passwords page's reveal/edit path — hooking
them would make *browsing your own vault* reshuffle your autofill order.

**Private-window use is not recorded.** A vault entry implies its site, so persisting "used login X"
from a private window leaves precisely the trace private browsing exists to avoid.

## Alternatives

- **Last-used as a global sort, ignoring match quality** (Bitwarden's extension behavior) — rejected:
  match strength is a *relevance* claim about this page and recency is not. It would sink a
  never-used exact-host match for `accounts.google.com` below a just-used domain-wide `google.com`
  entry, and it would break the `exact_host_ranks_above_domain_match` invariant (ADR-0016) for a
  case the tie-break already handles. Within-tier ordering fixes the reported complaint — several
  logins domain-matching one site — without touching that invariant.
- **Restricting the default list to exact-host matches, with search for the rest** — considered and
  rejected 2026-07-16: `Domain` is Bitwarden's per-URI default, so most real entries live in that
  tier; a Host-only list would hide the `google.com` login on `accounts.google.com` — the common
  case — behind a search field that does not exist yet. Popover search is filed separately (P2-42).
- **Storing usage inside the encrypted cache** (`vault.json`) — rejected: that blob is the verbatim
  server `/sync` response, so folding in a local field means restructuring the cache format and
  rewriting an encrypted document on every fill. The data protected would be opaque cipher UUIDs and
  timestamps — no secret, no site, no username. Cost far exceeds the exposure.
- **In-memory only** — rejected: the order would reset every launch, which is the complaint.
- **Sorting by a frecency score** (visit-count-weighted, like `history.rs`) — rejected as premature:
  plain recency is what the user asked for and what the mainstream password managers do. Frecency is
  a refinement to make if plain recency proves too jumpy.

## Consequences

- The popover and the in-page picker both improve; they share `service::lookup`, so the picker got
  the ordering for free. The Passwords page (`service::list`) is untouched — it is a manager, not a
  chooser.
- `matching_logins` and `VaultManager::lookup` grew a `usage` parameter. Taking it as an argument
  (rather than reading the global inside) keeps both pure and unit-testable, and keeps the
  process-global in `service`, where the other globals already live.
- `vault-usage.json` accumulates one entry per login ever used, and entries for deleted logins
  linger harmlessly (an id that matches nothing sorts nothing). No pruning: the map is bounded by
  the user's vault size.
- A user who fills exclusively from private windows sees no reordering. Intended.
- The at-rest exposure is a new file revealing that *some* vault item UUID was used at time T. No
  secret, no site, no username — asserted by a unit test on the serialized shape.
- Deleting `vault-usage.json` is a clean reset to pure vault order.
