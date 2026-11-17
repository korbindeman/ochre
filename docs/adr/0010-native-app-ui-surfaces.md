# ADR-0010: Internal app-UI surfaces are native per-platform views, not web pages

- **Status:** Accepted
- **Supersedes:** [ADR-0009](0009-settings-web-page.md)
- **Date:** 2026-07-06

## Context

[ADR-0009](0009-settings-web-page.md) chose core-served internal **web pages** for settings
(and, by extension, the other app-UI surfaces - history, bookmarks, downloads). It was
reopened on two axes it never weighed:

1. **Real Liquid Glass is impossible in a web page.** Genuine glass is compositor-level
   backdrop refraction - it samples what is physically behind the surface. A document
   rendered inside a CEF view cannot reach the native window content or desktop behind it;
   CSS `backdrop-filter` only blurs within the page's own document. So a web surface tops
   out at a flat frosted *approximation* with no true vibrancy. If Liquid Glass is the goal,
   it forces native. There is no web middle path.
2. **The thesis is "native OS chrome"** (CLAUDE.md) - each platform looking like *itself*.
   ADR-0009's headline benefit was pixel-identical cross-platform settings, but a shared web
   skin is exactly what this project positions *against*. Visual cross-platform sameness is
   therefore anti-thesis here, not a win; the parity that matters is *behavioral*, and the
   shared core already enforces that (ADR-0007).

There is also an architectural-continuity point. The app's existing surfaces - sidebar,
command bar, empty state - are already SwiftUI `NSHostingView` islands over FFI
([RootView.swift](../../apps/macos/Sources/RootView.swift)). A native app-UI panel is *that
same pattern*. The web path was the larger departure: it required standing up a whole new
substrate (custom scheme handler + `CefMessageRouter` bridge + an origin security boundary +
a from-scratch CSS design system + a JS/Vite toolchain bolted onto a Rust+Swift build).

## Decision

Internal **app-UI surfaces** - settings, history, bookmarks, downloads - are built as
**native per-platform views**: SwiftUI + Liquid Glass on macOS, WinUI 3 + Fluent on Windows
(future). They **render in the content area** - where web pages render - summoned like
opening a page (History via ⌘Y), not as `browser://` URLs and not as separate windows. Each
is a content-area **surface** shown over the tab content, modeled on the existing native empty
state ([RootView.swift](../../apps/macos/Sources/RootView.swift)) - **not a formal tab**: not
in the tab list, not juggleable, no web-style back/forward. Promoting them to first-class tabs
would mean teaching the core tab model about non-CEF (internal) tabs (synthetic ids, the event
stream, id→view reconciliation); deferred until juggleable internal tabs are actually needed,
and revisited then.

- **Logic and data stay in `crates/core`** behind FFI - unchanged from the project's
  "native layers are view + binding only" rule. Only the *view* is per-platform.
- **Internal web HTML is reserved for surfaces that must render inside the web viewport** -
  error pages (rendered in the CEF view on a failed load, with no native window to summon)
  and future reader-mode / PDF-style content. No scheme handler or JS↔native bridge is built
  for app UI.
- **Escape hatch:** if a single app-UI surface's scope grows large enough that a shared
  cross-platform web implementation is warranted - and its not-quite-native feel becomes an
  acceptable trade - revisit *that surface* and supersede this ADR for it. Native is the
  default until scope forces the question. Migration runs native → web if ever, never the
  reverse.

## Alternatives

- **Core-served web pages (ADR-0009)** - superseded: cannot deliver true Liquid Glass,
  imposes a large fixed substrate up front, and its cross-platform-visual-sameness benefit
  conflicts with the native-chrome thesis. The per-platform view duplication it optimized
  against is **view-only** (logic stays in core) and **deferred** (Windows is a later phase),
  so that cost is smaller and later than it appeared.
- **Web now, migrate to native later** - rejected: backwards. The substrate is the expensive
  part; building it only to throw it away is worse than building native views directly. If
  scope ever forces a change it goes native → web, per the escape hatch above.
- **Hand-rolled AppKit `NSToolbar` windows** - rejected (unchanged from ADR-0009): more code,
  gives up SwiftUI's free `Form`/`List` styling and Liquid Glass, no benefit.

## Consequences

- **No web-framework decision is needed** (Solid/Svelte/etc. - moot), and **no JS/Node
  toolchain** enters the build. No `CefMessageRouter` bridge, custom scheme handler, or
  origin security boundary is built for app UI - the entire substrate ADR-0009 implied is
  not built. (Error pages still use internal HTML, but one-way, no bridge.)
- **History is the first surface and sets the pattern.** The core work is the same it would
  have been under either approach: extend [`history.rs`](../../crates/core/src/history.rs)
  with a time-ordered listing + delete/clear + the FFI (today's `query` is autocomplete-
  shaped - frecency-ranked, no timestamps), then build a SwiftUI Liquid Glass view as the
  reference the other surfaces follow.
- **Windows later** reimplements the *view* in WinUI/Fluent; core logic + FFI are shared, so
  cross-frontend parity stays behavioral (core-enforced, ADR-0007), not pixel-identical -
  consistent with the thesis.
- **ADR-0008's "excluded from the web set" framing is moot** (there is no web set); its
  decision - no new-tab page - stands unchanged.
- This is a new UI/UX rule and should be reflected in `docs/style-guide.md` (app-UI surfaces
  are native Liquid Glass; web is for in-viewport content only) when the style guide is next
  touched.
