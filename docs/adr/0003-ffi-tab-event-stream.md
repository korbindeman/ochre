# ADR-0003: Rust↔Swift tab event stream over the hand-rolled C ABI

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

The multi-tab core (`mvp` M1 / backlog P1-M1) needs a **live, push** channel from the Rust core to
the macOS app: the core owns the tab/space model and must tell Swift when a tab is created, closed,
selected, or changes title / url / load-state / favicon. `mvp` M1 explicitly flags the observation
mechanism as "the decision to lock here," and suggests **swift-bridge** or a **UniFFI callback
interface** as the object-y boundary, noting plain request/response UniFFI is a poor fit for a stream.

Grounding the decision against the code as it actually stands (not the docs' aspiration):

- **No binding-generator is wired.** The `uniffi` in `crates/ffi/Cargo.toml`'s *description* is
  aspirational text - there is no `uniffi`/`swift-bridge` dependency, build-script, or codegen step
  anywhere in the workspace.
- **A working push channel already exists**, hand-rolled: `browser_set_find_result_callback` registers
  a C function pointer the core invokes on the main thread for find-in-page results. Swift consumes the
  core through a plain, hand-written bridging header mirroring the `#[no_mangle] extern "C"` surface.
- The tab event set is **small and stable** (≈8 event kinds), main-thread-only (CEF's UI thread is the
  app's main thread in embedded mode), and each event is a `(kind, id, optional string)` triple.

`CLAUDE.md` weighs directly on this: *"no dependency you don't need,"* *"three similar lines beat a
premature abstraction,"* keep the FFI *coarse-grained*.

## Decision

**Extend the existing hand-rolled C ABI.** Add one callback registration -
`browser_set_event_callback(cb)` - where the core invokes `cb(event_type: i32, id: i32, payload:
*const c_char)` on the main thread, exactly mirroring the find-result-callback pattern. Tab commands
are ordinary coarse C exports (`browser_tab_create` → returns the `tab_id`, `browser_tab_close`,
`browser_tab_select`, `browser_tab_move`). `tab_id` is **CEF's own browser identifier**
(`Browser::identifier()`), obtained synchronously from `browser_host_create_browser_sync`, so there is
no async create→event correlation to get wrong. Swift registers one top-level `@convention(c)` trampoline
that routes events into the observable `BrowserState`.

The event kinds (payload in parens): `TabCreated(url)`, `TabClosed`, `TabSelected` (`id = -1` ⇒ no
active tab / empty state), `TabTitleChanged(title)`, `TabUrlChanged(url)`, `TabLoadStarted`,
`TabLoadEnded`, `TabFaviconChanged(host)`. Space events join the same callback in P1-M4.

## Alternatives

- **swift-bridge** - rejected *for now*. It generates real Swift types + async sequences, which is
  genuinely nicer *if the surface grows large*, but it adds a dependency and a build-time codegen step,
  and would replace the clean hand-written bridging header. For an 8-event, `(i32, i32, string)` stream
  that is overhead the event set doesn't justify. Revisit if the FFI surface balloons (rich per-tab
  structs, bidirectional object graphs) - that's the trigger to reopen this ADR.
- **UniFFI callback interface** - rejected. Mozilla-mature, and the Cargo.toml already name-drops it,
  but it is request/response-oriented; async event *streams* are awkward in it, and it's still a new
  dependency + codegen. Poorest fit of the three for a live tab-update stream.
- **Poll from Swift** (no push; Swift asks the core for tab state on a timer) - rejected. Wasteful,
  laggy, and it reintroduces exactly the jank `mvp` risk #4 warns against.

## Consequences

- **We hand-maintain the event enum + marshalling on both sides** (the Rust dispatch and the Swift
  trampoline). Acceptable because the set is small and stable; the cost is a few parallel lines, not an
  abstraction. If the two sides drift, it's a compile/`switch`-default miss, not a silent corruption.
- **Zero new build complexity** - no codegen, no generated-source checked in, no dependency treadmill.
  Consistent with the coarse-FFI rule (`CLAUDE.md`) and the existing find-callback precedent.
- **`tab_id` = CEF browser identifier** couples our tab identity to CEF's. That is fine and convenient
  (stable for the browser's life, unique, integer), and it makes popups/DevTools - which also fire
  `on_after_created` - trivially *not* tabs, since only `browser_tab_create` registers a tab.
- **Sync creation** (`browser_host_create_browser_sync`) replaces the async
  `browser_host_create_browser` + `on_after_created` correlation for tabs. Safe because, with
  launch-to-empty, the first tab is created while the message loop is already pumping and the window is
  already visible.
- Supersedes the title-via-`NSWindow`-KVO hack (`BrowserState.pageTitle` fed from the window title):
  titles now arrive as `TabTitleChanged` events attributed to a specific `tab_id`.
