# ADR-0036: Self-tests normalize chrome state in-process - `$HOME` isolation does not cover NSUserDefaults

- **Status:** Accepted
- **Date:** 2026-07-09

## Context

The standing P0-1 regression `just focus-selftest` went red at HEAD (P1-29: the two
`sidebar-empty-click` cases) with no code change to blame - a bisect would have found every
commit red. Instrumenting the failing step showed the pinned sidebar host parked off-screen at
`x = -chromeWidth`: the app had launched with `isSidebarPinned == false`, so the synthetic click
at sidebar coordinates landed outside the window and hit nothing.

The selftest recipes launch the app with a throwaway `HOME="$(mktemp -d)"` to dodge Chromium's
process singleton, and the harness assumed that also meant a pristine preference state. It does
not: **NSUserDefaults resolves through cfprefsd against the logged-in user's account, not the
process's `$HOME`**, so a temp-HOME run still reads the developer's real
`com.nativechrome.Browser.dev` domain. The moment the developer unpinned the sidebar while
dogfooding (`sidebarPinned.dev = 0`), the harness's implicit precondition - a pinned sidebar
hosting the URL bar - silently broke. Any pref-shaped chrome state (viewport URL bar, future
toggles) leaks into "hermetic" runs the same way.

## Decision

Self-test harnesses **explicitly establish the chrome state their cases assume, in-process, as
their first step** - they never rely on launch-environment isolation for preference state.
`FocusSelfTest` starts with a `normalize-chrome-state` step that assigns
`isSidebarPinned = true` and `isViewportURLBarEnabled = false` on `BrowserState` directly.
Plain `@Published` assignments drive the real RootView layout paths but are never persisted
(only the user-facing toggle actions write UserDefaults), so the developer's real preferences
are untouched.

## Alternatives

- **Bisect for the breaking commit** - dead end; the failure is environment-dependent, not
  commit-dependent. Recorded here so the next red-at-HEAD selftest gets its environment checked
  (`defaults read com.nativechrome.Browser.dev`) before its history.
- **Blank the preference domain at selftest launch** (`removePersistentDomain`) - rejected: it
  deletes the developer's *real* preferences (cfprefsd points at the live domain, not a copy).
- **Override via `NSArgumentDomain` / registered defaults in the recipe** - rejected: requires
  enumerating every pref key (with its `.dev` profile suffix) in the `justfile`, which drifts as
  prefs land; a missed key fails the same silent way. The in-process step keeps the precondition
  next to the cases that need it.
- **Have the harness drive the user toggles (`toggleSidebar`)** - rejected: the toggles persist
  to UserDefaults, so a selftest run would rewrite the developer's real settings.

## Consequences

- `just focus-selftest` is hermetic w.r.t. developer preferences; green/red tracks the code again.
- Each new pref that changes chrome geometry must be added to the normalize step if the selftest's
  cases assume a particular value - the step is the declared list of assumed state.
- `just drag-selftest` currently passes with ambient prefs, but it inherits them the same way; if
  it ever goes red with no code suspect, normalize its preconditions the same way.
- The throwaway `$HOME` remains necessary (Chromium singleton, cache, session state) - it just
  isn't sufficient for prefs.
