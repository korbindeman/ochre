# ADR-0030: Appearance — forward the color scheme to web content, one Light/Dark/System setting

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

In the ALLOY (chromeless) hosting model (`spec §14`, `embed.rs`) CEF does **not** propagate the
macOS system appearance to web content: `matchMedia('(prefers-color-scheme: dark)')` reports
`light` even when the Mac is in dark mode, because CEF's Alloy runtime never wires Chromium's
`ui::NativeTheme` to the host `NSView`/`NSApplication` appearance. Result: dark-mode-aware sites
render their light theme inside a dark OS — a visible, confirmed defect (user-tested).

Two things are wanted together:
1. **Track the OS** — pages should see the system light/dark scheme by default.
2. **Let the user override it** — a Light / Dark / System choice, like every mainstream browser.

The lever exists on the request context: `CefRequestContext::SetChromeColorScheme(variant,
user_color)` with `ColorVariant::{SYSTEM, LIGHT, DARK}` (cef 148). Browsers created with no
explicit request context (as all of ours are — `embed::create_browser` passes `None`) share the
**global** context, so one call themes every tab.

## Decision

**One appearance preference — System / Light / Dark — owned by the core, driving both the web
content color scheme and the native chrome.**

- **Core** (`crates/core/src/theme.rs`) holds the preference, persisted to `theme.json` next to the
  CEF cache (same write-through pattern as `consent.rs`). It resolves the effective scheme and
  calls `request_context_get_global_context().set_chrome_color_scheme(LIGHT|DARK, 0)`.
- **System resolves in the core from a host-forwarded OS appearance**, not from CEF's `SYSTEM`
  variant. The host pushes `set_system_dark(is_dark)` on launch and on every OS theme change (KVO
  on `NSApp.effectiveAppearance`); in System mode the core re-applies the resolved LIGHT/DARK. We
  do **not** rely on `ColorVariant::SYSTEM` auto-tracking, because the same Alloy gap that causes
  the bug means "let CEF follow the OS" is exactly what is unproven — so we drive it explicitly and
  keep the mechanism observable.
- **The preference also drives `NSApp.appearance`** (Light→`aqua`, Dark→`darkAqua`, System→`nil`)
  so the native shell (sidebar, command bar, glass) shares the chosen theme. A theme that lit the
  pages but not the chrome (or vice-versa) would be a split-appearance bug.
- **Applied live, no reload.** `set_chrome_color_scheme` dispatches a native-theme change that
  updates `prefers-color-scheme` on already-loaded pages, so an OS dark-mode flip re-themes open
  tabs without a jarring reload (verified via the CDP harness before landing — the check that the
  Alloy gap is actually closed).
- Surfaced as **View → Appearance → System / Light / Dark** (radio checkmarks via
  `AppDelegate.validateMenuItem`), FFI `browser_theme_{preference,set_preference,set_system_dark}`.

## Alternatives

- **Per-page CDP `Emulation.setEmulatedMedia` (`prefers-color-scheme`)** — rejected: per-browser,
  must be re-issued on every navigation and every new tab, and requires the consent-gated CDP
  surface (P0-4) to be enabled. `SetChromeColorScheme` is one global call that all tabs inherit.
- **`--force-dark-mode` command-line switch** — rejected: it *auto-darkens* light pages
  (Chromium's rendering-level inversion), it is not a `prefers-color-scheme` signal — sites that
  ship a real dark theme wouldn't get it, and it can't express Light or track the OS.
- **Trust `ColorVariant::SYSTEM` to track the OS** — rejected as the mechanism for System mode: the
  Alloy runtime is precisely where OS-appearance propagation is unproven (it's the bug). Forwarding
  the resolved value from the host is deterministic and testable; we still pass through the same
  `set_chrome_color_scheme` path, so if a future CEF makes `SYSTEM` reliable we can drop the
  forwarding without an API change.
- **Theme web content only, leave native chrome on the OS setting** — rejected: Chrome and Arc both
  make one appearance choice govern the whole browser; a Dark web view inside a Light shell reads
  as a bug, not a feature (`style §2` consistency).
- **Store the preference in the Swift layer** — rejected: appearance is browser state, must persist
  identically for future frontends, and the resolution/apply logic is frontend-agnostic — it
  belongs in the core (CLAUDE.md: no logic a second frontend would reimplement).

## Consequences

- Every tab inherits the appearance for free (global request context); no per-browser bookkeeping.
- A clean host↔core handshake: the host owns OS-appearance *detection* (KVO) and native-chrome
  appearance; the core owns the *preference*, its persistence, and the CEF apply. Windows/Linux
  reuse the same core surface — they only add their platform's appearance detection + native theme.
- The core now calls a CEF request-context API on the UI thread; `theme::apply()` is main-thread
  only, like the other `embed` operations.
- If CEF/Chromium later moves the pin off the 148 line (ADR-0028), re-verify the color-scheme path
  — the `SetChromeColorScheme` semantics are the kind of thing an engine bump can shift.
- Forward-compat: the persisted `theme.json` is a struct, so a later per-site or scheduled
  (auto light/dark by time) override lands without a migration.
