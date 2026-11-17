# ADR-0056: Pinned URL is separate from live navigation

- **Status:** Accepted, refines [ADR-0038](0038-pinned-favorites-are-permanent.md)
- **Date:** 2026-08-14

## Context

Saved tabs and favorites had one `url` field serving two incompatible roles: the
durable pinned identity and the page currently loaded in CEF. Every
navigation therefore replaced the only URL the model knew, session persistence
made that replacement durable, and close had no home URL to restore.

ADR-0038 deliberately kept the current page of a favorite loaded on close. The
product direction now uses the pinned-URL behavior from Arc. Navigation away
from a pin is temporary. Only an explicit replacement moves the pin.

## Decision

Each saved tab and favorite has two URL concepts:

- `saved_url` is its durable pinned identity. Open tabs have no `saved_url`.
- `url` is the live page. It continues to drive the address bar, favicon, history,
  and content events.

Pinning an open tab captures its current URL. Moving between the saved section and
the favorite strip preserves the existing pin. Unpinning removes the pin and keeps
the current live page as an ordinary open tab.

Ordinary navigation changes only `url`. Closing an active saved tab resets `url`
to `saved_url`, clears temporary page metadata, deselects it, and suspends its CEF
browser. Closing a favorite performs the same reset and deselection. It then
navigates the resident hidden browser to `saved_url`. ADR-0038 still requires
the permanent icon and warm browser.

`session.json` keeps its existing `url` field. For open tabs, it stores the live
URL. For saved tabs and favorites, it stores `saved_url`. Temporary navigation
does not survive relaunch. No schema migration is required.

The only operation that changes an existing pin is `replace_pinned_url(id)`.
Native frontends show it as **Replace Pinned URL with Current**. The action is in
the tab context menu and the File menu while the live URL differs from the pin.

## Alternatives

- **Persist both live and pinned URLs.** Rejected because temporary navigation
  would return after relaunch. This result contradicts the requested "does not
  stick" behavior and adds stale session state.
- **Use browser history to find the pinned page.** Rejected because history has
  no durable boundary that identifies the pin. Redirects can also rewrite the
  history.
- **Replace the pin automatically on navigation.** Rejected because it recreates
  the original conflation and makes accidental navigation destructive.
- **Discard the browser of a favorite on close.** Rejected because favorites
  remain resident by ADR-0032/0038. A hidden navigation returns the warm browser
  home without removing it.

## Consequences

- A pin remains stable across navigation, crashes, and relaunches until the user
  explicitly replaces or removes it.
- Snapshot JSON gains nullable `saved_url` so native frontends can surface the
  replacement action without duplicating pin logic.
- Session JSON is backward-compatible. Legacy saved tabs restore with their stored
  URL as both the pinned and initial live URL.
- Close now causes a hidden home navigation for favorites. Network and page events
  may continue briefly after the favorite leaves the canvas.
