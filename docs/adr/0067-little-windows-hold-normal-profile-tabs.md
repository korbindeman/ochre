# ADR-0067: Little windows hold normal-profile tabs outside the sidebar

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

Arc's Little Arc is a small window that shows one tab. You open one with Cmd+Opt+N. Links from other apps open there, and Cmd+O moves the page into the main window. Arc also opens a link from a Favorite that leaves the Favorite's site in a quick-look peek, so the Favorite keeps its own page. Korbin wants the same UX across the app (P1-45). He reported that navigating on favorites "doesn't work well" (2026-11-18).

## Decision

- **A little window is a tab in the normal profile.** It uses the same cookies, history and adblock as the main window. It is not a private window.
- **The core keeps it in a separate list, `TabStore.little`, outside `tabs`.** No sidebar, active-tab, suspension, closed-stack or session code iterates that list, so a little tab cannot leak into those paths. Id lookups and content updates (title, URL, load state, favicon) cover both lists. The snapshot lists little tabs under `little_tabs`.
- **Little windows are not persisted.** A little window is a quick look, not part of the session. Closing one is not recorded for Cmd+Shift+T.
- **Cmd+O moves the tab, with its live browser, into the main window.** The tab becomes the last Open tab of the active Space and the active tab. The host moves the tab's CEF host view into the main window first, so the page is not reloaded.
- **Commands follow the key window.** A focused little window sets `FOCUSED_LITTLE`, and the single-target verbs (navigate, reload, back, zoom, find, DevTools, site shield) act on its tab. Any other window taking focus clears it.
- **Favorites stay themselves.** In `on_before_browse`, a user-clicked main-frame link in a Favorite that leaves the Favorite's registrable domain is cancelled and opens in a little window instead. Same-site navigation, redirects and script navigation stay in place.
- **Links from other apps open in a little window by default.** This is a core setting (`external_links_little`, default on), with a toggle in Settings.
- **Shortcuts:** New Little Window is Cmd+Opt+N (Arc). New Space moves to Ctrl+Cmd+N.

## Alternatives

- **A `Placement::Little` variant in the one tab list.** Rejected. About 40 sidebar and active-tab paths iterate `tabs`, and each would need a filter. One missed filter means a little tab shows up in a Space or gets suspended. A separate list makes that class of bug impossible.
- **Reload the URL in a new main-window tab on Cmd+O.** Rejected. This loses scroll position, form state and back history, and Arc keeps the page.
- **Peek as an overlay inside the main window instead of a separate window.** Deferred. The user asked for "the exact same UX everywhere", and one little-window surface for peeks and external links is the smaller change. Revisit if the separate window feels wrong in use.
- **A private-profile little window.** Rejected. Links from Mail or Slack need the user's signed-in session.

## Consequences

- Little windows reuse the main window's command bar for Cmd+L (route: current tab) and Cmd+T (new main-window tab).
- The property test covers opening, promoting and closing little tabs. It checks that a little tab is never a Space's active tab and that its id never collides with a sidebar tab.
- Moving a CEF host view between NSWindows is the one engine-side risk. It is verified by hand in the app, with no selftest yet.
