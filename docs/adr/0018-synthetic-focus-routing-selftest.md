# ADR-0018: Focus-routing regression — in-process synthetic-input self-test

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

P0-1 — keyboard/focus routing between CEF's `NSView` and the native chrome — is the gating risk for
all remaining MVP1 UI work (P1-M2/M3/M5 and the vault UI are `blocked` on it). It is the one seam a
screenshot can't prove: an occluded window renders blank yet routes input fine, and a rendered window
can still drop keystrokes on the floor. The routing was confirmed once by an interactive pass
(2026-07-06) and a fix landed for the edge case where a sidebar click leaves the bare `NSWindow` as
first responder (overlay close now drives CEF `set_focus` via `browser_focus_active_tab`). But an
interactive pass is not repeatable, and the CEF **security-bump treadmill** re-touches exactly this
surface on every engine update — so the routing needs a *standing, non-interactive* regression, not a
one-time human check.

The constraint that shapes the design: this project's verification harness is **headless** (isolated
`$HOME`, assert on stderr, never steal focus). Any regression must run with no human, no GUI
approval, and without the app being frontmost.

## Decision

A **debug, in-process synthetic-input self-test**, triggered by a `--focus-selftest` launch flag
(`apps/macos/Sources/FocusSelfTest.swift`). It opens a network-free `data:` fixture tab, then drives
a scripted sequence of focus transitions using **`NSApp`/`NSMenu`-level synthesized input** and
asserts, after each, that the main window's `firstResponder` is the active tab's CEF web view
(`isDescendant(of:)` the tab's container) — the in-process ground truth for "keys will land in the
page," corroborated by the core's existing `[focus] cef on_got_focus` stderr. It prints one
`[focus-selftest] case=… result=PASS/FAIL` line per case and exits non-zero on any failure, so
`just focus-selftest` is a green/red gate.

Input is synthesized at the app's own dispatch entry points, not the OS event stream:
- **Menu shortcuts** (`⌘T`, `⌘L`) via `NSApp.mainMenu.performKeyEquivalent(_:)` — the exact
  `performKeyEquivalent:` path the design relies on to intercept shortcuts *before* CEF.
- **Clicks** via `NSApp.sendEvent(_:)` with a synthesized `NSEvent` mouse-down/up — real hit-testing
  and first-responder handling into CEF's view.
- **The bare-`NSWindow` precondition** (the state a sidebar click produces) via
  `window.makeFirstResponder(nil)` — reproduced directly rather than by simulating the sidebar click,
  because the value under test is the *recovery* (overlay close refocuses the page), not AppKit's
  plain resign-on-click.

Cases: tab-switch focus, click-to-focus into the page, command-bar open/close refocus (common case),
the **bare-window regression** for the landed fix, and `⌘L` inline-URL-edit open/dismiss refocus.

## Alternatives

- **External `CGEventPost` / accessibility driver** — rejected. Posting OS-level events to another
  app requires the posting process to hold macOS Accessibility (AX) trust *and* the target to be
  frontmost. Neither is grantable in a headless/agent session (no GUI to approve AX; the app launches
  behind other windows), so the events would silently not deliver. A regression that only runs on a
  manually-permissioned, foregrounded desktop is not a standing regression. `NSApp.sendEvent`
  bypasses the window-server/CGEvent gate entirely and needs no AX.
- **XCUITest UI test target** — rejected. XCUITest drives via the accessibility API (same AX gating
  as above in a headless run) and would pull an Xcode test target + scheme into a project whose app
  must boot from a hand-assembled CEF `.app` bundle (framework + per-role helpers injected by
  `scripts/bundle-macos-app.sh`). The bundle-first launch path doesn't fit XCTest's runner; the cost
  and fragility outweigh the benefit for one seam.
- **Assert only on the core `[focus]` stderr log, no in-process check** — rejected as the *primary*
  signal. The logs are corroborating, but `window.firstResponder` read in-process is the direct
  truth ("which responder receives keys") and lets the harness compute a clean pass/fail + exit code
  without brittle log scraping. The stderr lines remain in the run artifact as a second channel.
- **Directly call the focus methods (`focus_active_tab`, `set_focus`) and skip synthesized input** —
  rejected. That tests the plumbing but not the *routing* — whether `⌘T` is actually intercepted by
  the menu before CEF, whether a click lands first responder on CEF's view. Synthesizing at the
  dispatch entry points is what makes it a routing test rather than a plumbing test.

## Consequences

- P0-1's proof becomes a repeatable command (`just focus-selftest`) that the CEF-bump treadmill
  re-runs — the routing can never silently regress again without turning the gate red.
- The harness is **debug/dev-only**, gated behind the `--focus-selftest` flag; it adds a small,
  flag-scoped code path and a few internal test-hook accessors on `AppDelegate`/`RootView`. It is not
  compiled out of release by `#if`, so if release builds later matter it should be gated then.
- One fidelity boundary, recorded honestly: the bare-`NSWindow` precondition is set programmatically
  (`makeFirstResponder(nil)`), and command-bar/`⌘L` *dismissal* is invoked via the controller's
  `hide()`/`endEditing()` rather than a synthesized Escape — because AppKit's local key monitors
  aren't driven by `NSApp.sendEvent`. The refocus logic under test runs identically; only the
  keystroke that triggers dismissal is not itself synthesized. Actual character-delivery *into* the
  page (vs. first-responder ownership) still wants a JS-eval readback or a real interactive tap — out
  of scope here; first-responder + `on_got_focus` is the accepted proof per the P0-1 plan.
