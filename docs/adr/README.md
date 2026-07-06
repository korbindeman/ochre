# Architecture Decision Records

This directory is the project's **decision log** — the persistent, git-committed record of
*why* the browser is built the way it is. `CLAUDE.md` and `docs/architecture.md` describe
*what* the system is; ADRs capture the decisions behind it, including the alternatives that were
rejected and why. Read here before reopening a settled choice.

## How to use

- **Read first.** Before making or revisiting a non-trivial design choice, skim the index below
  (and the relevant ADRs) so you don't re-litigate a settled question or re-explore a rejected path.
- **Add one when a decision is made** — whenever you choose among alternatives, defer or cut scope,
  or reverse an earlier approach (architectural or not). Do it at decision time; the reasoning is
  otherwise lost. Copy `0000-template.md` to `NNNN-kebab-title.md` (next number in sequence), fill
  it in, and add a row to the index.
- **One decision per record.** Keep each ADR short and focused.
- **Immutable once Accepted.** Don't rewrite an accepted decision — write a new ADR that supersedes
  it, set the new one's `Status` to reference the old, and flip the old ADR's `Status` to
  `Superseded by ADR-NNNN`. Update the index.
- **Commit ADRs alongside the code** that realizes the decision (ideally one ADR per PR).

## Index

| ADR | Title | Status |
|-----|-------|--------|
| [0001](0001-roadmap-steering-harness.md) | Roadmap steering harness — strategy / execution split + `steer` skill | Accepted |
| [0002](0002-tweaks-page-customization.md) | Tweaks — one page-customization engine, three tiers, on the adblock injection path | Accepted |
| [0003](0003-ffi-tab-event-stream.md) | Rust↔Swift tab event stream over the hand-rolled C ABI | Accepted |
| [0004](0004-sidebar-url-chip.md) | Sidebar URL chip — a persistent current-tab address display | Superseded by [0014](0014-inline-sidebar-url-editing.md) |
| [0005](0005-structure-snapshot-events.md) | Structure changes ship as a snapshot re-pull, not fine-grained events | Accepted |
| [0006](0006-spaces-folders-transforms.md) | Spaces, folders, pinned/ephemeral tabs, and the folder↔space transform | Accepted |
| [0007](0007-parity-ledger.md) | Cross-frontend parity as a status-only ledger classified by the consistency contract | Accepted |
| [0008](0008-no-new-tab-page.md) | No new-tab page — new tab is the command bar; not-browsing is the native empty state | Accepted |
| [0009](0009-settings-web-page.md) | Settings UI is a core-served internal web page (custom scheme + `CefMessageRouter`), not a native panel | Superseded by [0010](0010-native-app-ui-surfaces.md) |
| [0010](0010-native-app-ui-surfaces.md) | Internal app-UI surfaces (settings, history, …) are native per-platform views (Liquid Glass), not web pages | Accepted |
| [0011](0011-omnibox-fuzzy-ranking-inline-autocomplete.md) | Command-bar autocomplete — purpose-built fuzzy ranking + inline autocomplete | Accepted |
| [0012](0012-traffic-lights-placeholder-hover.md) | Traffic lights — custom-drawn in the sidebar, native buttons hidden | Accepted |
| [0013](0013-session-persistence-lazy-restore.md) | Session persistence — write-through JSON + lazy tab hydration, core tab ids decoupled from CEF | Accepted |
| [0014](0014-inline-sidebar-url-editing.md) | Inline sidebar URL editing — the chip edits in place, not via the command bar | Accepted |
| [0015](0015-tab-drag-reorder.md) | Tab drag-and-drop — implicit order, one relocation primitive, commit-on-drop | Accepted |
