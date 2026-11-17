# ADR-0009: Settings UI is a core-served internal web page, not a native panel

- **Status:** Superseded by [ADR-0010](0010-native-app-ui-surfaces.md)
- **Date:** 2026-07-06

## Context

Settings is a needed surface (roadmap §4 Tier 1, §5.3). How to build it is a real fork,
because this app's structure removes the obvious default.

Apple's premade answer is SwiftUI's `Settings { … }` scene - it auto-creates the settings
window, the **Settings…** menu item, and the ⌘, binding. But it is a *`Scene`*, and Scenes
only exist inside a SwiftUI `App` lifecycle. This app has none: `main.swift` runs top-level
AppKit code that lets **CEF install its own `NSApplication` subclass and own `NSApp`** before
any AppKit touches the shared app, then hands the run loop to `browser_run_message_loop()`
([main.swift](../../apps/macos/Sources/main.swift:7)). SwiftUI is used only inside
`NSHostingView` islands. So the premade solution is off the table, and AppKit never shipped a
first-class preferences window either - you host it yourself regardless.

That leaves two real shapes: a **native SwiftUI panel** hosted in a dedicated `NSWindow`, or a
**core-served internal web page**. For a browser the native panel is arguably the odd choice -
Chrome, Edge, Firefox, and Arc all do settings as an internal page - and the project's two
load-bearing principles push the same way: settings *values* are core-owned regardless, and a
shared HTML page is identical on macOS and Windows, serving the cross-platform consistency
contract (ADR-0007) instead of creating two native panels that drift.

## Decision

**Settings is a core-served internal web page.**

- **Served from `crates/core`** embedded resources via a **custom scheme** (`browser://settings`),
  so it has a real origin and works with the address bar and back/forward - not a `data:` URL or
  `LoadString`.
- **State is core-owned** (Rust). The page reads and writes it over a **JS↔native bridge** -
  CEF's `CefMessageRouter` (the `window.cefQuery` request/response pattern): page reads current
  values, page flips a setting, core pushes a change back.
- **The native layer contributes only** the **Settings…** menu item (⌘,) that navigates to
  `browser://settings`, plus the native escape-hatch handlers below, invoked *through* the bridge.

This reverses the "native SwiftUI panel for MVP" lean floated earlier in this discussion.

## Alternatives

- **SwiftUI `Settings` scene (Apple's premade)** - rejected: unavailable. It requires the
  SwiftUI `App` lifecycle, which conflicts with CEF owning `NSApp` and driving the run loop.
  Adopting it means fighting CEF for the application object.
- **Native SwiftUI panel in a dedicated `NSWindow`** (`Form` + `TabView`) - rejected as the
  owner of settings: it is the lighter *first* build, but it must be reimplemented in WinUI on
  Windows, violating "never put logic in a frontend a second frontend reimplements," and it
  maximizes drift against the consistency contract. Wrong long-term owner even though it's the
  cheaper start.
- **Hand-rolled AppKit `NSToolbar` preferences window** - rejected: the most code, gives up
  SwiftUI's free `Form` styling, and buys no cross-platform benefit.

## Consequences

- **Establishes the internal-page substrate** - a custom scheme handler + a `CefMessageRouter`
  bridge. This is reusable infrastructure that history, bookmarks, and downloads are candidates
  to inherit, but those are **decided separately**, not here. ALLOY custom error pages already do
  internal-HTML *rendering* but not the *bridge*, so settings is the first bridge customer unless
  a simpler surface lands first. The cost shape is therefore **"bridge + page," not "page"** - the
  substrate is front-loaded, but it is infrastructure wanted regardless.
- **Security boundary (non-negotiable):** the bridge must gate privileged settings calls
  (`set_default_search_engine`, etc.) on the requesting frame's **origin** - only `browser://`
  internal pages, never arbitrary web content. Get this wrong and any site can rewrite the user's
  settings.
- **Not purely web.** OS-integrated rows round-trip through the bridge to native: download folder
  → `NSOpenPanel`, make-default → `LSSetDefaultHandler`, permission rows → deep-link into System
  Settings. There is a thin, deliberate native seam.
- **Look/feel:** the page will not be native chrome - acceptable because settings is *content*,
  not chrome (sidebar, command bar, traffic lights stay native). But the page must itself honor
  system appearance - `prefers-color-scheme`, reduce-motion, increased-contrast (style guide:
  honor system settings) - since the OS won't apply it to web content.
- **Cross-platform:** Windows inherits the identical settings UI for free - the strongest
  expression of the consistency contract (ADR-0007).
- If a future change wants OS-native settings behavior the web page can't express, extend the
  bridge with another native handler rather than forking settings back into a native panel;
  supersede this ADR only if the web-page model itself stops fitting.
