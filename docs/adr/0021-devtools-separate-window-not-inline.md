# ADR-0021: DevTools opens as a separate window, not docked inline

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

DevTools should ideally dock *inline* inside the browser window (a resizable panel
attached to the page, bottom/right, like Chrome/Safari/Arc) rather than opening as
a separate floating window. This was investigated end-to-end against our actual
engine, CEF 149 (Chromium 149), and found **not feasible** for our architecture.

Two hard CEF facts collide:

1. **Our tabs are Alloy-style, deliberately.** The whole embedding model (spec's
   "Option B", ADR-era `embed.rs`) hands CEF a child `NSView` so the native app
   owns the window and draws its own Arc-style chrome. That requires **Alloy**
   runtime style per browser — Chrome style would bring Chrome's own tab strip /
   omnibox, which we don't want.

2. **Under the Chrome bootstrap (M128+, which 149 is), a DevTools browser must be
   Chrome-style.** Creating one Alloy-style is rejected outright:
   `ERROR ... browser_host_create.cc: Alloy style is not supported for this browser`.

To render DevTools into a sibling `NSView` you must pass a `parent_view` to
`ShowDevTools`/`CefWindowInfo::SetAsChild` — but the macOS CEF header states
verbatim *"Alloy style will always be used if `parent_view` is provided."* So
`SetAsChild` forces Alloy, which DevTools rejects → a hard Chromium `CHECK`
(SIGTRAP) inside `show_dev_tools`. Confirmed empirically.

The fallback — let CEF open DevTools in its own Chrome-style window and **adopt its
render `NSView`** into our pane, hiding the transient window — also fails the bar:

- `CefWindowInfo` is **ignored** for Chrome-bootstrap DevTools popups. Even with
  `use_default_window = 0` and `hidden = 1`, CEF creates and **shows** its own
  window at a default position `(100,100,640,640)`. It can't be created hidden or
  off-screen.
- `BrowserHost::GetWindowHandle()` returns `NULL` in the DevTools browser's
  `on_after_created` — the platform view doesn't exist yet — so the view can only
  be grabbed *after* the window has already appeared on screen.

Net: a visible DevTools window (a flash, at minimum) is unavoidable. The product
requirement was an inline panel with **no** stray window visible anywhere (not on
screen, not in the Dock icon's window menu); that can't be met.

## Decision

**Keep DevTools as a separate CEF-managed window** — `ShowDevTools(None, …)`,
Chrome-style, its own top-level window. Triggered by ⌘⌥I (menu), F12 (engine-side
keyboard handler), and right-click *Inspect Element* (focused on the element), all
of which were already working. Do not attempt inline docking on CEF 149.

## Alternatives

- **Child `NSView` docking (`ShowDevTools` with `SetAsChild`)** — rejected: forces
  Alloy style on the DevTools browser, which the Chrome bootstrap rejects → CHECK
  crash. This is the "clean" approach and it is simply not available here.
- **View adoption (open DevTools windowed, reparent its `NSView` into our pane,
  hide the source window)** — rejected: `CefWindowInfo` is ignored so the window
  can't be created hidden/off-screen, and `GetWindowHandle()` is null until after
  the window is shown — so a visible window/flash is unavoidable, violating the
  "no stray window" requirement. (Implemented and tested; the transient
  `DevTools - example.com` window stayed on screen at `(100,100,640,640)`.)
- **Tracked borderless child window pinned over a placeholder pane** ("fake dock")
  — rejected by the user on UX grounds: focusing DevTools makes it a separate *key*
  window, so the main window's title bar reads inactive while typing in the console.
- **Make tabs Chrome-style so DevTools docks natively** — rejected: Chrome style
  brings Chrome's own window UI (tab strip, toolbar), which defeats the native
  Arc-style chrome that is the point of the product. This is a whole-architecture
  change, not a DevTools tweak.

## Consequences

- DevTools works and is discoverable (menu item + ⌘⌥I + F12 + Inspect Element), just
  as a separate window — the mainstream-but-not-inline behavior.
- No dead scaffolding: the docked-pane UI (split pane, dock-side switch, resize,
  header controls, view-adoption engine code) was fully removed rather than left
  half-wired.
- This is a **CEF-version constraint**, not a permanent law. If a future CEF gains
  Alloy-style DevTools or embeddable Chrome-style DevTools views (see CEF issues
  #3681 "lightweight Alloy-style windows/browsers" and #3294 "embedded non-Views
  windows"), revisit and supersede this ADR. Re-run the check by pointing
  `ShowDevTools` at a child `NSView` and watching for the "Alloy style is not
  supported" error.
- Windows/Linux frontends inherit the same engine constraint; don't promise inline
  DevTools there either until CEF changes.
