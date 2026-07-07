# ADR-0020: Native ad-blocking — request-path engine, seed-plus-hot-swap lists, per-site shield UI

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

`spec §11` and the roadmap (`rm §1.2`, §3.1) commit to native, request-path ad-blocking via
Brave's `adblock` crate (adblock-rust) as one of the three defensible differentiators — the thing
MV3 extension blockers structurally can't do. That the *engine* is adblock-rust was already settled;
this ADR records the decisions made when wiring it end-to-end into CEF with the necessary UI, which
were **not** obvious defaults:

- **How the engine attaches to CEF**, and on which thread it runs.
- **Where filter lists come from** — a browser must block on first launch (offline) yet stay current.
- **How "turn it off for this site" is scoped** — the per-site control users expect.
- **What ships now vs. later** — cosmetic (element-hiding) filtering needs a document-start injection
  substrate this project doesn't have yet (P2-14, shared with vault autofill and Tweaks).

The user asked for adblock "end to end, including the proper necessary UI." Scope was confirmed
interactively (2026-07-07): **network blocking + full control UI now; cosmetic as a fast-follow.**

## Decision

**1. Request-path hook (network blocking).** A `RequestHandler` on the CEF client returns a
`ResourceRequestHandler` whose `on_before_resource_load` returns `ReturnValue::CANCEL` for
ad/tracker requests (`crates/core/src/client.rs`). The block decision comes from a process-global
adblock engine (`crates/core/src/adblock/`). CEF resource types are mapped to adblock-rust's
request-type strings; the top-level document URL (`Request::first_party_for_cookies`) is the
first-party for third-party detection and the host the allowlist keys on. **Top-level document
(`MAIN_FRAME`) loads are never cancelled** — blocking a navigation would blank the tab; adblock
targets sub-resources.

**2. Threading.** `on_before_resource_load` runs on the CEF **IO thread**, and a background thread
hot-swaps a freshly-downloaded engine in, so the engine lives behind `RwLock<Arc<Engine>>` and the
per-tab blocked counters behind a `Mutex`. `check` clones the `Arc` and drops the read lock before
matching, so a hot-swap never waits on a match. This forces the crate's `single-thread` feature
**off** (it's what makes `Engine: Send + Sync`, asserted by the crate).

**3. Domain resolver.** adblock's `embedded-domain-resolver` feature is **off**; we register a
`psl`-backed `ResolvesDomain` at startup — reusing the Public Suffix List crate already compiled for
the vault instead of pulling in adblock's separate `addr` crate (spec §11's guidance, and genuinely
one fewer public-suffix dependency).

**4. Filter-list sourcing — seed + background hot-swap.** A small, curated **seed list** of
high-confidence ad/tracker hosts is compiled into the binary (`adblock/lists/seed.txt`,
`include_str!`) and built synchronously at startup, so blocking is live and offline on the very first
navigation. A background thread then upgrades to the cached full lists (**EasyList + EasyPrivacy**)
and refreshes them from upstream when the cache is missing or > 3 days old, hot-swapping the engine
in place. Cache is plain combined list text (`adblock-lists.txt`), re-parsed on a background thread
at launch — simple and format-version-free; a serialized-engine (DAT) fast-load is a noted perf
follow-up.

**5. Per-site allowlist keyed on the registrable domain.** "Block Ads on This Site" adds the page's
eTLD+1 (via `psl`) to a persisted allowlist (`adblock.json`, mirroring `cdp.rs`), so turning blocking
off for a site covers its subdomains but never leaks across a public suffix. Global on/off lives in
the same file; blocking is **on by default**. Toggling either reloads the active tab so it applies on
screen (matching Brave/Arc).

**6. UI — a sidebar shield + menu toggles (P1-18).** A shield glyph with the live blocked count sits
beside the sidebar URL chip; clicking it opens a native glass popover with the count and a per-site
"Block Ads on This Site" toggle. Global and per-site toggles also appear as native menu items
(every feature gets a menu path). The shield reads the **active tab's** count/state through the FFI
(`browser_adblock_active_*`), resolved in the core from `tabs::active_browser()` — so the FFI never
has to translate core tab ids to CEF ids. No filter-list settings page yet (deliberately minimal per
the user); a management surface stays Phase-2 settings-UI territory. UI treatment is `style §16`.

**7. Cosmetic filtering deferred to P2-21.** Element-hiding CSS + scriptlets need injection at
**document-start** to avoid a flash of the un-hidden element (FOUC); doing that right is the shared
content-script injection substrate (P2-14, also used by vault autofill and Tweaks). Injecting
hide-CSS at load-end would flash — a half-version — so cosmetic filtering waits for that substrate
rather than shipping degraded.

## Alternatives

- **Cosmetic filtering now, via the existing load-end `execute_java_script` path** — rejected:
  injecting element-hiding CSS at `on_load_end` flashes the ad before hiding it, and would fork a
  parallel injection mechanism from the one P2-14 will build for autofill/Tweaks. Wait for the
  document-start substrate and build it once (P2-21).
- **`single-thread` engine feature on (spec §11's perf hint)** — rejected: it makes `Engine`
  `!Sync`, which breaks sharing the engine across the IO thread and a background hot-swap thread. The
  micro-perf is not worth building the engine on, and confining it to, a single thread; correctness
  of the hot-swap wins.
- **`embedded-domain-resolver` on (the default, zero setup)** — rejected: it compiles adblock's
  `addr` public-suffix crate *in addition* to our `psl`. A 3-line `psl` resolver removes the
  redundant dependency and keeps one PSL implementation.
- **Download lists on first run with no seed (or blocking fetch before first paint)** — rejected: a
  no-seed first launch has no blocking until the network fetch lands (weak first impression), and a
  blocking fetch delays startup and fails offline. The compiled-in seed makes launch instant and
  offline-correct; the hot-swap upgrades transparently.
- **Bundle the full EasyList/EasyPrivacy snapshots in-repo** — rejected: multi-MB generated files
  rot in git and bloat the repo; the seed + background-update model keeps lists fresh without
  vendoring them. (The seed is small and curated, not a full list.)
- **Per-tab `RequestContext` per site** — rejected as unnecessary: all tabs share the global request
  context and the handler filters per-URL against the shared engine + allowlist. Per-context isolation
  is the spaces/containers primitive (`rm §5.3`), not an adblock concern.
- **Block `MAIN_FRAME` loads too** — rejected for v1: cancelling a top-level navigation blanks the
  tab (false-positive risk far outweighs the value); adblock's value is in sub-resources.
- **Key blocked counts by CEF browser id across the FFI** — rejected: core tab ids are decoupled from
  CEF ids (ADR-0013), so the FFI would have to translate. Exposing the **active tab's** count/state
  (what the sidebar chip already represents) avoids translation entirely.

## Consequences

- **A working, visible adblocker now:** ad/tracker network requests are cancelled on the request path,
  the seed makes it effective offline from first launch, full lists hot-swap in within seconds, and the
  shield surfaces the per-page block count with a per-site off switch. Closes P0-3's request-path exit
  criterion (`rm §5.0`).
- **Cosmetic filtering (P2-21) is now gated on the injection substrate (P2-14).** Until it lands,
  blocked ads can leave empty layout gaps that network blocking alone doesn't collapse — the known,
  documented boundary of this pass (not a bug).
- **A standing list-freshness dependency:** the background refresh hits easylist.to; failures are
  logged and retried next launch (the seed/cache keeps working). Filter lists join the things the
  security-treadmill cadence should sanity-check.
- **Follow-ups filed, not dropped:** cosmetic + scriptlets (P2-21); DAT fast-load cache; honoring
  `redirect`/`removeparam` results (v1 handles outright blocks only); a filter-list settings/management
  surface when Phase-2 settings-UI lands.
- **Hot-path cost:** every non-main-frame request takes an `RwLock` read + an Arc clone + an engine
  match on the IO thread. adblock-rust is built for exactly this (it powers Brave); the clone-and-drop
  keeps the lock uncontended against hot-swaps.
