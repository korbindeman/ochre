# ADR-0051: Favicon fetch runs off-thread behind an SSRF guard

- **Status:** Accepted
- **Date:** 2026-07-16

## Context

Favicons are fetched by `crates/core/src/client/favicon.rs` with `ureq`, in the
main browser process, outside CEF's network stack (so the fetch bypasses the
adblock engine and any proxy). The candidate URLs come partly from the page's
own `<link rel="icon" href="...">` tags, which are attacker-controlled.

Two problems followed from fetching those URLs verbatim:

1. **SSRF.** A page could declare `<link rel="icon" href="http://169.254.169.254/...">`
   (cloud metadata), `http://127.0.0.1:8080/admin` (a loopback service), or a LAN
   router endpoint. On load the privileged process issued a blind GET there,
   reachable from the user's network position, bypassing adblock. `ureq` follows
   redirects by default, so even a public-looking favicon could 3xx onto an
   internal host. Only the *well-known fallback* paths were pinned to the page
   origin; the *declared* URLs had no scheme or host restriction. Surfaced by the
   comprehensive security audit (2026-07-16).
2. **UI-thread stall.** The fetch was synchronous blocking HTTP on the CEF UI
   thread, so a slow/tarpit favicon host froze the whole browser, re-triggerable
   by any page.

The real decision was *how* to constrain the destination without breaking two
legitimate cases: cross-origin CDN favicons (e.g. github.com serving its icon
from `github.githubassets.com`) and a user visiting their own local-dev server on
loopback (which must still get its own favicon).

## Decision

- **SSRF guard (`fetch_url_allowed`).** A favicon target is fetched only if: the
  scheme is `http`/`https`; and either the target host equals the page's own host
  (same-origin, so a loopback dev page still resolves its favicon), or the host
  resolves *entirely* to publicly-routable addresses. Internal ranges are
  rejected: loopback, RFC-1918 private, link-local (incl. `169.254.169.254`),
  CGNAT `100.64/10`, IPv6 unique-local/link-local, unspecified, broadcast,
  multicast. IPv4-mapped IPv6 is judged as its v4 address.
- **Redirects are followed by us, not `ureq`** (`max_redirects(0)`), re-running
  the guard on every hop, so a public first hop can't bounce onto an internal
  host.
- **Off the UI thread.** `start_favicon_download` runs the download on a worker
  thread with explicit connect + global timeouts, then marshals the store +
  tab-notify back to the UI thread via `post_task` (the notify emits a
  `tabs::Event` the native layer expects on its main actor).
- Added `url` as a direct core dependency (already compiled in via `adblock`, so
  no new crate) for robust scheme/host parsing on a security boundary, instead of
  hand-rolled string splitting.

## Alternatives

- **Disable redirects entirely** - rejected: closes the redirect-SSRF bypass with
  no code, but favicons that legitimately 3xx (e.g. `/favicon.ico` → a CDN) would
  silently fail, a visible favicon-coverage regression. Per-hop revalidation keeps
  coverage without the risk.
- **Block all cross-origin fetches** - rejected: would break the common CDN-hosted
  favicon pattern. The threat is internal-host reach, not cross-origin per se.
- **Connect-by-resolved-IP to close the DNS-rebinding gap** - deferred: the guard
  resolves for its check and `ureq` resolves again to connect, so a rebinding
  attacker could in principle race the two. Closing it fully needs a custom
  connector. Disproportionate here: this is a *blind* GET (no response body ever
  reaches the page), and the load-bearing wins - IP-literal targets and redirect
  bounces to internal hosts - are fully closed. Documented as an accepted residual
  in the code.
- **Hand-rolled URL host parsing** - rejected: error-prone on a security boundary
  (userinfo `@`, IPv6 brackets, ports). Used the `url` crate.

## Consequences

- A malicious page can no longer drive the browser into blind requests against
  cloud-metadata / loopback / LAN hosts via favicon links, and can no longer hang
  the UI with a slow favicon host.
- Favicon capture is now asynchronous: it completes a beat after load (worker
  thread → `post_task`), which is invisible in practice and already how the tab
  list updates (event-driven snapshot re-pull).
- Accepted residual: DNS-rebinding against the guard for a blind GET (see
  Alternatives). If a non-blind favicon path is ever added, revisit
  connect-by-IP.
- Guarded by unit tests (`ssrf_guard_*`, `public_ip_classification`) and the
  existing hermetic `just favicon-selftest` (same-origin loopback, which the guard
  must keep allowing).
