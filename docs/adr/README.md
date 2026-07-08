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
| [0016](0016-password-vault-bitwarden-crypto-first.md) | Password vault — Bitwarden-compatible (rbw-derived) client, built core-crypto-first | Accepted |
| [0017](0017-vault-autofill-injected-content-script.md) | Vault autofill — injected isolated-world content script + in-page picker, native save banner | Accepted |
| [0018](0018-synthetic-focus-routing-selftest.md) | Focus-routing regression — in-process synthetic-input self-test (`--focus-selftest`) | Accepted |
| [0019](0019-sidebar-glass-material-beside-content.md) | Sidebar glass — standard sidebar material beside content, not lensing over the CEF view | Accepted |
| [0020](0020-adblock-request-path-engine.md) | Native ad-blocking — request-path engine, seed + hot-swap lists, per-site shield UI | Accepted |
| [0021](0021-devtools-separate-window-not-inline.md) | DevTools opens as a separate window — inline docking isn't feasible under CEF 149 (Alloy tabs) | Accepted |
| [0022](0022-surface-tab-mutual-exclusivity.md) | App-UI surfaces and tabs are mutually exclusive at the UI layer (highlight suppressed, close on select), not via a core deselect | Accepted |
| [0023](0023-icon-button-feedback-style.md) | Chrome icon buttons share one `IconButtonStyle` — uniform hover + press feedback, replacing hand-rolled per-button highlights | Accepted |
| [0024](0024-vault-email-two-factor-send-flow.md) | Email login 2FA — client-triggered `send-email-login`, provider tag carried UI→core, auto-send on challenge | Accepted |
| [0025](0025-bitwarden-cloud-default-provider.md) | Bitwarden cloud is the default provider (no URL); self-hosted Vaultwarden behind a toggle; `for_input` host resolution | Accepted |
| [0026](0026-cookie-consent-reject-first.md) | Cookie-consent — reject-first (Consent-O-Matic + TCF), hide as an honest long-tail fallback; forces an all-frames substrate extension | Accepted |
| [0027](0027-oopif-injection-blocked-by-cef-devtools.md) | Cross-origin iframe (OOPIF) injection is blocked by CEF's DevTools observer (relays child-session results, not events); same-origin ships, OOPIF needs a render-process path (P2-24) | Accepted |
| [0028](0028-pin-cef-148-subresource-redirect-regression.md) | Pin CEF to the 148 line — Chromium 149 drops subresource redirects (`ERR_INVALID_ARGUMENT`); broke the Google avatar | Accepted |
| [0029](0029-core-initiated-main-world-fill.md) | Button-driven vault autofill fills via a core-initiated main-world evaluate (main frame, no context tracking); the isolated-world bridge stays reply-only | Accepted |
| [0030](0030-appearance-color-scheme-forwarding.md) | Appearance — forward the color scheme to web content via the global request context; one Light/Dark/System setting drives web content + native chrome | Accepted |
| [0031](0031-private-windows-incognito.md) | Private windows — incognito `RequestContext` per window, ephemeral tabs, privacy gates, opt-in vault fill | Accepted |
| [0032](0032-pinned-tabs-favorites.md) | Pinned tabs (favorites) — Arc-style resident icon strip, dismiss-not-destroy, distinct from saved | Accepted |
| [0033](0033-runtime-tab-suspension.md) | Runtime tab suspension — reuse lazy hydration, grace-delayed background discard | Accepted |
| [0034](0034-commands-browser-automation.md) | Commands — user-defined browser automation, unified in the command bar; ⌘K palette additive | Accepted |
