# ADR-0008: No new-tab page — new tab is the command bar, not-browsing is the native empty state

- **Status:** Accepted
- **Date:** 2026-07-06

> **Note ([ADR-0010](0010-native-app-ui-surfaces.md)):** This ADR was written when other
> internal surfaces were planned as core-served *web pages* (ADR-0009). ADR-0010 reversed
> that — those surfaces are now native views. This ADR's decision (*no new-tab page*) is
> unaffected; only its "excluded from the web set" framing is moot, since there is no web set.

## Context

The roadmap listed "new tab page" as a Tier-1 core surface (§4, §4.1, §5.3), inherited
from the mainstream-browser checklist where a new tab lands on a `chrome://newtab`
dashboard (search box + top sites).

But this project copies **Arc's interaction model**, and Arc has no such page. Two
surfaces already cover everything a new-tab page would:

- **"New tab" is the command bar.** ⌘T opens the transient command bar, not a page
  ([Menu.swift:80](../../apps/macos/Sources/Menu.swift:80)); entering a URL or query
  there opens the tab. This is Arc's ⌘T, and it already *is* the "search box you get
  when you want a new tab."
- **The not-browsing state is a native island.** When no tab is active the content
  area shows `EmptyStateView`, a SwiftUI island AppKit owns
  ([RootView.swift:24](../../apps/macos/Sources/RootView.swift:24)) — not a web page.

A `browser://newtab` HTML page would be a *third* surface duplicating both. The
question was sharpened by the parallel decision to serve other internal surfaces
(settings, later history/bookmarks/downloads) as core-owned web pages: it forces us to
say explicitly whether new-tab is one of those, or excluded from that set.

## Decision

**There is no new-tab page.** No `browser://newtab` URL exists; nothing navigates to
one. The "new tab / not browsing" experience is jointly and solely:

1. the **command bar** (⌘T) as the entry point, and
2. the native **`EmptyStateView`** island as the not-browsing surface.

New-tab is therefore **excluded** from the set of internal surfaces that become
core-served web pages. Any future enrichment of the not-browsing state (recent tabs,
top sites, a greeting) goes *into the existing `EmptyStateView` island*, not a new
"page" concept.

## Alternatives

- **A `chrome://newtab`-style web page** (search box + top-site tiles) — rejected: it
  duplicates the command bar, which is already our Arc-model new-tab entry point, and
  competes with the native empty state for the same job. Two surfaces doing one thing.
- **A richer *native* new-tab dashboard** (top sites / recents as its own surface) —
  rejected: that is just growing the empty state. If we ever want that content it
  belongs in the `EmptyStateView` island under one owner, not a separate surface with
  its own lifecycle.
- **Leaving it on the roadmap ("decide later")** — rejected: it was a phantom scope
  item, and worse, an open invitation for a future agent to add the redundant surface
  the Arc model deliberately omits. Cutting it now removes the ambiguity.

## Consequences

- The command bar and `EmptyStateView` are the **single owners** of the new-tab /
  not-browsing experience; enrichment lands in the empty-state island. Keeping them the
  only owners is the thing to protect.
- Roadmap references struck in three places (Tier 1 list §4, Minimum Viable Daily
  Driver §4.1, Core surfaces §5.3).
- **Cross-platform:** Windows inherits the same model — new tab = command bar,
  not-browsing = a native empty state — so there is no new-tab surface for a second
  frontend to reimplement.
- If a future direction genuinely wants a web new-tab page (e.g. a shared,
  core-served dashboard across frontends), **supersede this ADR** rather than quietly
  adding a `browser://newtab` surface alongside the two that already cover the job.
