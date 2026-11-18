# ADR-0072: Move to system webviews behind a Rust engine trait, gated on a WKWebView spike

- **Status:** Accepted
- **Date:** 2026-11-18
- **Supersedes:** ADR-0066

## Context

ADR-0066 kept CEF on the codec and DRM question. Korbin reopened it on a different axis: Ochre is
a personal browser run by one person and a few agents, and CEF makes us own Chromium's security
treadmill (a release every four weeks, security respins every week or two, the custom codec build
of ADR-0040 on every bump). The pin to 148 (ADR-0028) already means we run without newer Chromium
security fixes. The goal is a good, secure, private browsing experience per platform with the
security work carried by the OS vendor. A different engine (Ladybird) may be wanted later.

## Decision

1. **Priority:** low maintenance and vendor-patched security come before Chromium compatibility
   and CDP-level hooks.
2. **Engine per platform:** WKWebView on macOS; later WebView2 on Windows and WebKitGTK on Linux.
3. **Engine glue lives in the Rust core** behind a narrow engine trait, with WKWebView driven from
   Rust through `objc2-web-kit`. A future vitae frontend or a Ladybird backend plugs into the same
   trait. The trait is shaped by the engines we have, not by a Ladybird that cannot embed yet.
4. **Ad blocking on WebKit is three layers:** `adblock-rust` lists compiled to WebKit content-rule
   JSON (the Brave iOS pipeline); scriptlets and cosmetic filters as `WKUserScript` in the page
   world (this is also the YouTube answer); an in-process proxy via
   `WKWebsiteDataStore.proxyConfigurations` as a hostname-level catch-all.
5. **CDP is dropped** on WebKit. Autofill and Commands move to injected scripts plus a message
   bridge. A cross-engine automation layer is wanted eventually but is low priority (backlog).
6. **Gate:** nothing is migrated until a spike (P0-5) shows WKWebView holds up on Korbin's own
   sites, video and FairPlay playback, and block-list coverage.

## Alternatives

- **Stay on CEF (ADR-0066)** - rejected: it keeps the security treadmill and the custom Chromium
  build on one person, which is the cost this decision exists to remove.
- **Swift owns WKWebView, core receives events** - rejected: puts engine logic in a frontend that
  the vitae frontend would have to reimplement.
- **`wry` (Tauri's webview crate)** - rejected: built for app webviews; lacks browser hooks
  (downloads, permissions, new-window policy) and would sit between us and the engine.
- **Decrypting HTTPS in the local proxy** (own root certificate, full-URL decisions) - rejected:
  breaks TLS for the whole browser; a bug or leaked key makes Ochre less secure than no blocker.
- **Overriding WKWebView's `https` scheme handling** (private behavior) - rejected: breaks cookies
  and service workers and any WebKit update can break it.
- **Migrate without a spike** - rejected: compatibility and blocking quality are the real risks and
  only Korbin's own browsing answers them.

## Consequences

- Retires on success: the custom codec build (ADR-0040/0042), the CEF pin (ADR-0028), the
  Chromium treadmill (`rm §8.1`), and the bundled Chromium. DRM becomes FairPlay through WebKit.
- Loses: Chromium site compatibility, request-level rewriting of headers and bodies, some filter
  options WebKit rules cannot express, CDP and the automation it enabled, extensions (already out).
- The migration touches roughly 9k of the core's 22k lines (`embed.rs`, `client/*`, `inject.rs`,
  `tabs.rs`, `private_window.rs`, favicon, devtools) plus the selftests: about 1.5 to 3M tokens.
- If the spike fails, this ADR is superseded and CEF stays.
