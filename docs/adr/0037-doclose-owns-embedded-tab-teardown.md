# ADR-0037: `DoClose` suppresses CEF's parent-window close for host-embedded tabs

- **Status:** Accepted
- **Date:** 2026-07-09

## Context

⌘W on an active **saved/pinned** tab sometimes closed the whole window instead of
suspending the tab to the empty canvas. The mechanism: our `LifeSpanHandler::do_close`
returned `false` for every browser, and CEF's documented contract for a windowed Alloy
browser is that returning `false` sends the **standard platform close to the browser's
top-level parent window** — `performClose:` on macOS (`cef_life_span_handler.h`). For a
tab embedded in the host's `NSWindow`, that is the *app window*: the `performClose:`
lands in `windowShouldClose` → `browser_request_close_all()` → graceful app quit.

The reason ⌘W on ordinary tabs never showed this: `DoClose` "will not be called if the
browser's host window/view has already been destroyed", and the Swift close path
(`RootView.closeTab`) detaches the CEF child view synchronously right after
`browser_tab_close` — the detach beats CEF's close pipeline. But every path that closes a
browser while its view is still attached was racing: `hide_saved_tab` (⌘W on an active
saved tab; detach rides the async `TabSuspended` → Combine hop), ADR-0033 background
suspension, bulk closes (clear-ephemeral, space delete), and a page calling
`window.close()` in a tab. Whenever CEF won, the window closed.

## Decision

`BrowserHandler` tracks **host-embedded tab browsers** (a `HashSet` of CEF ids,
registered by the synchronous tab-creation paths in `embed::create_tab_browser` and
`private_window::create_tab_browser`, dropped in `on_before_close`). `do_close` returns:

- **`true` for embedded tabs outside app shutdown** — CEF never touches the host
  window; the host's view detach (which every close/suspend path already performs)
  is what completes browser destruction. `do_close` also calls
  `tabs::note_closed` / `private_window::note_closed` so a page that closed *itself*
  still gets its tab row dropped → the host prunes the view → destruction completes
  (idempotent: host-initiated closes already removed the tab or took its binding).
- **`false` during app shutdown (`is_closing`)** — CEF's close re-request is what lets
  the host's vetoed `windowShouldClose` proceed; the quit flow is unchanged.
- **`false` for non-embedded browsers (popups, DevTools)** — CEF created their windows,
  and the default close is what closes them; OAuth popups rely on `window.close()`.

Realizing this exposed a second defect the fix depends on: `BrowserClient::create` built
a **fresh `BrowserHandler` per client** — one per tab — while `HANDLER_INSTANCE` kept
pointing at the first (the "replacing a viable instance" assert passes vacuously because
a `new_cyclic` weak has strong-count 0 during construction). Everything written against
the handler as shared state was silently split per tab: `close_all_browsers` drained one
browser, `browser_count` counted one list, `mark_closing` marked a handler no other
browser's callbacks consulted, and `embedded_tabs` registration landed in a handler
`do_close` never read. `BrowserClient::create` now reuses the live instance and only
constructs a handler on first use, making the shared state actually shared.

Additionally, ⌘W semantics on the empty canvas now match Arc: with no active tab, ⌘W
closes the window **even when saved tabs / favorites remain in the sidebar** (they
persist in the session and return on relaunch). The previous `tabs.isEmpty` guard meant
a user with any saved tab could never ⌘W-close the window. The key-repeat guard stays,
so hold-⌘W stops at the empty canvas (and only inspects `isARepeat` on `.keyDown`
events — CRASH-0002).

The whole path is guarded by a standing regression, **`just close-selftest`**
(`CloseSelfTest.swift`): ⌘W through the real menu dispatch against an ordinary tab, an
active saved tab (the suspend path that raced CEF), and the empty canvas — asserting the
window survives the first two and closes from the third.

## Alternatives

- **Detach the view synchronously in the hide/suspend paths too** (mirror
  `closeTab`). Rejected: fixes only the reported repro; every present and future
  close-with-attached-view path (suspension, bulk close, JS self-close) keeps racing
  CEF's pipeline. Correctness by winning a race is not a design.
- **Distinguish in `windowShouldClose`** (ignore CEF-initiated `performClose:`).
  Rejected: AppKit gives no way to tell CEF's `performClose:` from the user's — and
  the app-quit flow *depends* on CEF's re-request reaching `windowShouldClose`.
- **Track popups instead of tabs** (invert the set). Rejected: the popup browser's id
  is unknown at `on_before_popup` time; the tab ids are known exactly at our own
  synchronous creation sites. Tabs are also the set with the invariant we rely on
  ("host always detaches the view").
- **Look up "is a tab" in the tab store at `DoClose` time.** Rejected: the store
  binding is intentionally already gone by then (`close_tab` removes the tab,
  `hide_saved`/`suspend_tab` take the browser) — exactly the paths being fixed.

## Consequences

- Tab closes/suspensions can no longer close the window, regardless of view-detach
  timing; the empty canvas is always reached first.
- A tab page calling `window.close()` now closes just that tab (previously it could
  close the whole window — same race).
- The host's obligation is explicit: for every embedded browser it must eventually
  detach the CEF child view after a close/suspend, or destruction stalls. All current
  paths do (sync detach on close; `TabSuspended` and `StructureChanged`-prune on the
  rest; `windowWillClose` detaches all).
- `native_shell.rs` and the standalone spike path are untouched (their browsers are
  never registered as embedded tabs, keeping CEF's default close).
