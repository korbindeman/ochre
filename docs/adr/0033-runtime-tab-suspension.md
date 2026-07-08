# ADR-0033: Runtime tab suspension — reuse lazy hydration, grace-delayed background discard

- **Status:** Accepted
- **Date:** 2026-07-08

## Context

ADR-0013 gave every tab an `Option<Browser>` and a suspend/hydrate path for **session
restore** and **reopen closed tab**: restored tabs start with `browser: None` and the host
calls `browser_tab_restore` the first time they are shown. That saves renderer processes at
launch, but **runtime** background tabs still keep a live CEF browser after the user visits
them — `was_hidden` stops painting but not memory. MVP risk #3 (`mvp1.md`) and roadmap §8.2
call for a real per-tab memory policy before tab counts get real.

The suspend/hydrate machinery already exists; runtime suspension is the policy layer on top:
**discard** a background tab's browser (free the renderer), keep url/title in the core model,
**re-hydrate** on next show via the same `restore_tab` path.

## Decision

- **Reuse ADR-0013's `browser: None` state** for runtime suspension — no second "discarded"
  flag. `suspended_url` + `attach_browser` + `browser_tab_restore` are the re-entry path.
- **Auto-suspend policy (v1):**
  - **Never suspend:** the visible tab, favorites (ADR-0032 — resident strip), or during
    app shutdown (`begin_shutdown`).
  - **Tab switch:** schedule suspend of the tab the user left after a **30 s grace** (cancels
    if they return before it fires — avoids thrash on quick tab-hopping).
  - **Space switch:** **immediately** suspend every other hydrated tab in the space being left
    (except favorites).
- **Detach before close:** `suspend_tab` clears `tab.browser` *before* calling
  `host.close_browser`, so `on_before_close` → `note_closed` does not remove the tab (CEF id
  no longer maps once the handle is cleared).
- **New event `TabSuspended` (type 8):** lightweight signal for the host to detach the CEF
  `NSView` without a full `StructureChanged` reload. Snapshot adds `hydrated: bool` for sidebar
  dimming.
- **Settings / manual suspend:** deferred — v1 is automatic only; per-space hibernation knobs
  (roadmap §5.3) land later.

## Alternatives

- **Separate "discarded" bit alongside `browser: None`** — rejected: one suspended state is
  enough; url/title persistence already covers both restore-at-launch and discard-at-runtime.
- **`StructureChanged` on suspend** — rejected: full tree rebuild + spring animation on every
  background discard is noisy; a dedicated event is cheaper and matches ADR-0005's split
  (structure vs. content).
- **Immediate suspend on every tab switch (no grace)** — rejected for v1: fast back-and-forth
  would reload pages constantly; 30 s is a conservative default, tunable later.
- **Chromium `Page.discard` via CDP** — rejected for now: no CDP wiring in the product path;
  tearing down the CEF browser achieves the same memory win with code we already have.

## Consequences

- Background tabs visited once no longer retain a renderer indefinitely; memory scales with
  "warm" tabs (visible + favorites + grace-window leftovers) rather than session breadth.
- Revisiting a suspended tab reloads from the saved URL (history within the page is lost,
  same as Arc/Vivaldi hibernation). SPA state is not preserved — acceptable for v1.
- `TabSuspended` is the ninth event kind; hand-maintained on both sides per ADR-0003.
- Session write-through is unchanged; suspended tabs already persist url/title.
- A follow-up can add settings (grace duration, disable auto-suspend) and manual
  "Suspend Other Tabs" without revisiting the core detach path.