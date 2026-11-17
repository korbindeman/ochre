# ADR-0005: Structure changes ship as a snapshot re-pull, not fine-grained events

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

ADR-0003 gave the tab model a hand-rolled C-ABI event stream where each change is one
`(event_type, id, payload)` callback. That fit a **flat** tab list. M4 turns it into a **tree** -
space → folders → tabs - with **pinned/ephemeral** tabs (ADR-0006) and **bidirectional folder↔space
transforms**. A single transform reparents *many* tabs across spaces at once (a folder's tabs become a
new space; a space's pinned tabs become a folder elsewhere while its ephemeral tabs are closed).

Expressing that over fine-grained events means a `TabMoved` carrying three ids (tab, new space, new
folder) - more than the `(id, payload)` shape holds - emitted in a careful order the host must replay
without ever rendering a half-moved tree. That is brittle exactly where it's hard to debug.

## Decision

**Split the stream by change rate.** Keep the *hot, per-tab content* events fine-grained (they fire
often and touch one field): `TabTitleChanged`, `TabUrlChanged`, `TabLoadStarted`, `TabLoadEnded`,
`TabFaviconChanged`. Collapse *every structural change* - tab create/close, selection, pin/unpin,
folder membership, space/folder CRUD, and the transforms - into a single coarse **`StructureChanged`**
signal. On it, the host calls **`browser_structure_json()`** and rebuilds its tree from the snapshot
(spaces, folders, tabs with `space`/`folder`/`pinned`/order, the active space, each space's active tab).
Still the hand-rolled C ABI of ADR-0003 - this refines it, it doesn't reverse it.

## Alternatives

- **Fine-grained move events with a widened `(id, aux, payload)` signature** - rejected. A cross-space
  move needs three ids; transforms emit a burst that is order-dependent and easy to get subtly wrong.
  The snapshot is one code path that is always internally consistent.
- **Snapshot for *everything*, including content** - rejected. Titles and load-state churn continuously;
  re-pulling the whole tree on every title tick is wasteful. Content stays fine-grained.
- **A real serialization bridge (swift-bridge/UniFFI) for the tree** - rejected for the same reasons as
  ADR-0003: the snapshot is a single JSON string over the existing coarse FFI; no new dependency.

## Consequences

- Structural correctness is centralized: the core mutates its tree, emits one signal, and the host
  mirrors it wholesale - no partial-update ordering bugs.
- Re-pull cost is trivial (a handful of tabs/spaces per snapshot). The host **merges** the snapshot with
  its existing tab views to preserve per-tab UI state (e.g. favicon version) and avoid reload churn.
- `RootView` still creates a tab's CEF view synchronously from `browser_tab_create`'s return; the
  following `StructureChanged` just reconciles the model. Selection latency stays fine (snapshot is fast).
- The event enum is renumbered (content 0–4, `StructureChanged` 5); the bridging header is the source of
  truth for the wire values.
