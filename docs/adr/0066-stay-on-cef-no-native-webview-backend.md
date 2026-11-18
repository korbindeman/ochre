# ADR-0066: Stay on CEF; no native-webview engine backend

- **Status:** Superseded by ADR-0071
- **Date:** 2026-11-18

## Context

Korbin asked whether the core should be separated from the engine so Ochre could run on a native
webview (WKWebView on macOS) instead of only CEF, mainly for codecs and DRM. CEF today needs a custom
codec build (ADR-0040, ADR-0042, with an accepted AAC patent exposure), and premium DRM is blocked by
Widevine licensing (`rm §3.2`).

## Decision

**Stay on CEF. Do not build an engine abstraction or a WKWebView backend.**

## Alternatives

- **WKWebView backend behind an engine abstraction** - rejected for now. Gains: Apple's licensed
  codecs with hardware decode (would retire the custom CEF build), FairPlay instead of Widevine (likely
  removes the DRM licensing wall; not verified for a third-party WKWebView app), lower memory and no
  bundled Chromium. Losses: request-path ad blocking becomes declarative content rules only; the CDP
  layer (`cdp.rs`, autofill injection ADR-0017/0029, Commands ADR-0034, the planned AI layer) has no
  WebKit equivalent; worse compatibility for friends coming from Chromium-based Arc; thinner hooks for
  downloads, permissions, and requests. Passkeys gain nothing (the same Apple entitlement gate,
  ADR-0061). The abstraction itself means rebuilding `client/*`, `embed.rs`, `inject.rs`, `cdp.rs`,
  and the adblock path behind a trait, then carrying two engines' bugs (against ADR-0065).
- **Switch to WKWebView outright** - rejected for the same losses, without even the fallback.

## Consequences

- Codecs stay on the custom CEF build (ADR-0040/0042); DRM stays a labeled limitation (`rm §3.2`).
- Reopen trigger: if missing DRM becomes the friends' main complaint (ADR-0063), first run a small
  WKWebView spike (FairPlay playback on Netflix/Disney+, content-rule coverage of the current block
  lists, site breakage) before any refactor.
