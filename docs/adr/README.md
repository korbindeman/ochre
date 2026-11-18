# Architecture Decision Records

This directory is the project's **decision log** - the persistent, git-committed record of
*why* the browser is built the way it is. `CLAUDE.md` and `docs/architecture.md` describe
*what* the system is; ADRs capture the decisions behind it, including the alternatives that were
rejected and why. Read here before reopening a settled choice.

## How to use

- **Read first.** Before making or revisiting a non-trivial design choice, skim the index below
  (and the relevant ADRs) so you don't re-litigate a settled question or re-explore a rejected path.
- **Add one when a decision is made** - whenever you choose among alternatives, defer or cut scope,
  or reverse an earlier approach (architectural or not). Do it at decision time; the reasoning is
  otherwise lost. Copy `0000-template.md` to `NNNN-kebab-title.md` (next number in sequence), fill
  it in, and add a row to the index.
- **One decision per record.** Keep each ADR short and focused.
- **Immutable once Accepted.** Don't rewrite an accepted decision - write a new ADR that supersedes
  it, set the new one's `Status` to reference the old, and flip the old ADR's `Status` to
  `Superseded by ADR-NNNN`. Update the index.
- **Commit ADRs alongside the code** that realizes the decision (ideally one ADR per PR).

## Index

| ADR | Title | Status |
|-----|-------|--------|
| [0001](0001-roadmap-steering-harness.md) | Roadmap steering harness - strategy / execution split + `steer` skill | Accepted |
| [0002](0002-tweaks-page-customization.md) | Tweaks - one page-customization engine, three tiers, on the adblock injection path | Accepted |
| [0003](0003-ffi-tab-event-stream.md) | Rust↔Swift tab event stream over the hand-rolled C ABI | Accepted |
| [0004](0004-sidebar-url-chip.md) | Sidebar URL chip - a persistent current-tab address display | Superseded by [0014](0014-inline-sidebar-url-editing.md) |
| [0005](0005-structure-snapshot-events.md) | Structure changes ship as a snapshot re-pull, not fine-grained events | Accepted |
| [0006](0006-spaces-folders-transforms.md) | Spaces, folders, pinned/ephemeral tabs, and the folder↔space transform | Accepted |
| [0007](0007-parity-ledger.md) | Cross-frontend parity as a status-only ledger classified by the consistency contract | Accepted |
| [0008](0008-no-new-tab-page.md) | No new-tab page - new tab is the command bar; not-browsing is the native empty state | Accepted |
| [0009](0009-settings-web-page.md) | Settings UI is a core-served internal web page (custom scheme + `CefMessageRouter`), not a native panel | Superseded by [0010](0010-native-app-ui-surfaces.md) |
| [0010](0010-native-app-ui-surfaces.md) | Internal app-UI surfaces (settings, history, …) are native per-platform views (Liquid Glass), not web pages | Accepted |
| [0011](0011-omnibox-fuzzy-ranking-inline-autocomplete.md) | Command-bar autocomplete - purpose-built fuzzy ranking + inline autocomplete | Accepted |
| [0012](0012-traffic-lights-placeholder-hover.md) | Traffic lights - custom-drawn in the sidebar, native buttons hidden | Accepted |
| [0013](0013-session-persistence-lazy-restore.md) | Session persistence - write-through JSON + lazy tab hydration, core tab ids decoupled from CEF | Accepted |
| [0014](0014-inline-sidebar-url-editing.md) | Inline sidebar URL editing - the chip edits in place, not via the command bar | Accepted |
| [0015](0015-tab-drag-reorder.md) | Tab drag-and-drop - implicit order, one relocation primitive, commit-on-drop | Accepted (gesture layer superseded by ADR-0035) |
| [0016](0016-password-vault-bitwarden-crypto-first.md) | Password vault - Bitwarden-compatible (rbw-derived) client, built core-crypto-first | Accepted |
| [0017](0017-vault-autofill-injected-content-script.md) | Vault autofill - injected isolated-world content script + in-page picker, native save banner | Accepted |
| [0018](0018-synthetic-focus-routing-selftest.md) | Focus-routing regression - in-process synthetic-input self-test (`--focus-selftest`) | Accepted |
| [0019](0019-sidebar-glass-material-beside-content.md) | Sidebar glass - standard sidebar material beside content, not lensing over the CEF view | Accepted |
| [0020](0020-adblock-request-path-engine.md) | Native ad-blocking - request-path engine, seed + hot-swap lists, per-site shield UI | Accepted |
| [0021](0021-devtools-separate-window-not-inline.md) | DevTools opens as a separate window - inline docking isn't feasible under CEF 149 (Alloy tabs) | Accepted |
| [0022](0022-surface-tab-mutual-exclusivity.md) | App-UI surfaces and tabs are mutually exclusive at the UI layer (highlight suppressed, close on select), not via a core deselect | Accepted |
| [0023](0023-icon-button-feedback-style.md) | Chrome icon buttons share one `IconButtonStyle` - uniform hover + press feedback, replacing hand-rolled per-button highlights | Accepted |
| [0024](0024-vault-email-two-factor-send-flow.md) | Email login 2FA - client-triggered `send-email-login`, provider tag carried UI→core, auto-send on challenge | Accepted |
| [0025](0025-bitwarden-cloud-default-provider.md) | Bitwarden cloud is the default provider (no URL); self-hosted Vaultwarden behind a toggle; `for_input` host resolution | Accepted |
| [0026](0026-cookie-consent-reject-first.md) | Cookie-consent - reject-first (Consent-O-Matic + TCF), hide as an honest long-tail fallback; forces an all-frames substrate extension | Accepted |
| [0027](0027-oopif-injection-blocked-by-cef-devtools.md) | Cross-origin iframe (OOPIF) injection is blocked by CEF's DevTools observer (relays child-session results, not events); same-origin ships, OOPIF needs a render-process path (P2-24) | Accepted |
| [0028](0028-pin-cef-148-subresource-redirect-regression.md) | Pin CEF to the 148 line - Chromium 149 drops subresource redirects (`ERR_INVALID_ARGUMENT`); broke the Google avatar | Accepted |
| [0029](0029-core-initiated-main-world-fill.md) | Button-driven vault autofill fills via a core-initiated main-world evaluate (main frame, no context tracking); the isolated-world bridge stays reply-only | Accepted |
| [0030](0030-appearance-color-scheme-forwarding.md) | Appearance - forward the color scheme to web content via the global request context; one Light/Dark/System setting drives web content + native chrome | Accepted |
| [0031](0031-private-windows-incognito.md) | Private windows - incognito `RequestContext` per window, ephemeral tabs, privacy gates, opt-in vault fill | Accepted |
| [0032](0032-pinned-tabs-favorites.md) | Pinned tabs (favorites) - Arc-style resident icon strip, distinct from saved | Accepted (dismiss semantics superseded by ADR-0038) |
| [0033](0033-runtime-tab-suspension.md) | Runtime tab suspension - reuse lazy hydration, grace-delayed background discard | Accepted |
| [0034](0034-commands-browser-automation.md) | Commands - user-defined browser automation, unified in the command bar; ⌘K palette additive | Accepted |
| [0035](0035-custom-gesture-sidebar-drag.md) | Sidebar drag-and-drop - custom gesture-driven layer, one resolver, no system drag session | Accepted |
| [0036](0036-selftest-ambient-userdefaults.md) | Self-tests normalize chrome state in-process - temp `$HOME` does not isolate NSUserDefaults (cfprefsd reads the real user domain) | Accepted |
| [0037](0037-doclose-owns-embedded-tab-teardown.md) | `DoClose` returns true for host-embedded tabs - CEF's default close would `performClose:` the app window; ⌘W on the empty canvas closes the window (Arc) | Accepted |
| [0038](0038-pinned-favorites-are-permanent.md) | Pinned favorites are permanent - close deselects (never dismisses), the `dismissed` flag/ellipsis/−-button are gone; only Unpin/Delete remove one | Accepted (close reset refined by ADR-0056) |
| [0039](0039-folder-expansion-persisted-active-tab-peek.md) | Folder expansion lives in the core tree (persisted, survives space switches); a collapsed folder peeks the active tab so the current tab is never hidden | Accepted |
| [0040](0040-custom-cef-build-proprietary-codecs.md) | Custom CEF build with proprietary codecs (H.264/AAC) on an ephemeral cloud Mac - revises the rm §3.2 OS-decoder posture; AAC exposure accepted at pre-launch scale | Accepted |
| [0041](0041-sidebar-direct-swap-crossfades-in-place.md) | Pinned↔floating sidebar swap cross-fades in place (frozen geometry, matched alpha) instead of sliding two near-identical panels past each other; other transitions keep their slide | Accepted |
| [0042](0042-public-builds-ship-proprietary-codecs.md) | Public builds ship proprietary codecs - conscious pre-launch risk; maintainer-codecs / contributor-codecless GHA split; codec-less is the one-flag retreat (supplements ADR-0040) | Accepted |
| [0043](0043-space-switch-slides-per-space-content.md) | A space switch slides the sidebar as a horizontal filmstrip (all spaces mounted, offset animates to the active one; only the active panel is interactive) - deterministic direction, immune to `.id`-transition capture quirks; favorites strip + switcher stay put | Accepted |
| [0044](0044-favorites-active-per-space.md) | A favorite's active state is per-space, decoupled from its home `space` - selecting a global favorite shows it on the current canvas without switching spaces (fixes "pinned tabs jump to the first space"); refines ADR-0032/0038 | Accepted |
| [0045](0045-suspended-tabs-not-dimmed.md) | Suspended tabs render identically in the sidebar - the "not loaded" dimming is removed (read as a defect, flickered on show); reverses the dimming pre-committed in ADR-0013/0033, suspension mechanism unchanged | Accepted |
| [0046](0046-pinned-favorite-favicon-tint.md) | Pinned favorites get a persistent chip - neutral floor for presence + favicon-dominant-color wash for identity (saturation-gated, monochrome → neutral); extraction in Swift for now, promote to core for Windows; refines ADR-0032/0038 | Accepted (active ring superseded by ADR-0053; rest tint refined by ADR-0059) |
| [0047](0047-windows-parallel-frontend-track.md) | Windows is a parallel frontend track, gated only on the shared core - not sequenced behind Phase 4; `parity.md`'s Win column is the build-spec, no separate execution doc for now | Superseded by [ADR-0064](0064-vitae-frontend-replaces-swift-at-proven-parity.md) |
| [0048](0048-autofill-list-orders-by-last-used.md) | The autofill list orders by last-used **within** a match tier (never across it) - local plaintext `vault-usage.json`, recorded only on explicit fill/copy gestures, never from a private window or a Passwords-page reveal; refines ADR-0016/0029 | Accepted |
| [0049](0049-vault-search-is-core-owned-and-fuzzy.md) | Vault search is **one** core entry point (`vault::search`) shared by the popover's new search field and the Passwords page - fuzzy via the omnibox scorer, recency-tiebroken, empty query ⇒ whole vault; deletes the Swift-side filter rather than growing a second semantic | Accepted |
| [0050](0050-local-network-prompt-webrtc-mdns.md) | The macOS local-network prompt is triggered by WebRTC's mDNS local-IP obfuscation (verified; Cast discovery is inert under ALLOY) - keep the obfuscation for privacy and accept the one-time prompt; Media Router disabled as intent + CEF-bump guard, re-enable only with a full Cast slice (P2-44) | Accepted |
| [0051](0051-favicon-fetch-ssrf-guard.md) | Favicon fetch (privileged-process `ureq`, page-declared URLs) runs off the UI thread behind an SSRF guard - only http/https, same-origin or public-IP-resolving hosts, guard re-run per redirect hop; DNS-rebinding residual accepted for the blind GET; adds the `url` dep | Accepted |
| [0052](0052-auto-update-zinnia-model.md) | Auto-update - Zinnia model: GitHub `latest.json` + minisign-signed `Browser.app.tar.gz`, core pipeline, sidebar "Update & Restart" card; Sparkle stays the signed-production end-state (rm §6.2) | Accepted |
| [0053](0053-pinned-favorite-local-selection-indicator.md) | Pinned favorite selection is a local accent indicator - stable glass, no active wash, ring, or cross-tile slide | Superseded by ADR-0054 |
| [0054](0054-pinned-favorite-radial-selection-bloom.md) | Pinned favorite selection is a saturated radial site-color bloom beneath stable glass, animated locally with no cross-tile slide | Superseded by ADR-0055 |
| [0055](0055-pinned-favorite-inset-edge-bloom.md) | Pinned favorite selection is a saturated, rounded perimeter bloom that expands inward beneath stable glass | Superseded by [ADR-0059](0059-pinned-favorite-glass-means-selected.md) |
| [0056](0056-pinned-url-separate-from-live-navigation.md) | Pinned URL is separate from live navigation - close returns home, only explicit replace moves the pin | Accepted (refines ADR-0038) |
| [0057](0057-arc-pin-favorite-semantics.md) | Arc taxonomy: Favorite is global, Pinned is per-Space, Open is working state; pinned close never deletes; Pinned sections persist collapse state | Accepted (refines ADR-0032/0038/0056) |
| [0058](0058-native-menu-projects-browser-commands-into-cmd-t.md) | Cmd-T projects enabled native browser menu actions plus dynamic Space commands; user TypeScript Commands remain core-owned | Accepted (refines ADR-0034 built-ins) |
| [0059](0059-pinned-favorite-glass-means-selected.md) | Pinned favorite selection is the glass chip; idle tiles are a quiet fill (glass means selected, matching the tab list) | Accepted (supersedes ADR-0055) |
| [0060](0060-product-identity-vs-code-identifiers.md) | Product identity (`product.toml`) is separate from code identifiers (`browser-core`, `BrowserState`, Xcode scheme `Browser`); user-facing surfaces follow the toml | Accepted |
| [0061](0061-device-passkeys-os-platform-authenticator.md) | Device passkeys use the OS platform authenticator (iCloud Keychain); unsigned builds disable WebAuthn to avoid CRASH-0003; vault passkeys later | Accepted (ad-hoc keychain-access-groups stamp superseded by ADR-0062) |
| [0062](0062-adhoc-must-not-stamp-restricted-entitlements.md) | Ad-hoc debug signatures must not carry restricted entitlements; AMFI SIGKILLs `keychain-access-groups` on `codesign --sign -` (CRASH-0004) | Accepted |
| [0063](0063-personal-first-browser-for-arc-refugees.md) | Ochre is a personal browser first, for Korbin and friends leaving Arc; public launch parked; friends get signed GitHub Release builds with auto-update | Accepted |
| [0064](0064-vitae-frontend-replaces-swift-at-proven-parity.md) | SwiftUI is the only frontend now; a vitae (Rust) frontend comes later, once vitae renders native UI, and replaces Swift when shared selftests and screenshot comparisons prove parity | Accepted (supersedes ADR-0047) |
| [0065](0065-every-fix-adds-a-mechanical-guard.md) | Every fix adds a mechanical guard (lint, verify check, test or selftest); core invariants get property tests; repo stays on git | Accepted |
| [0066](0066-stay-on-cef-no-native-webview-backend.md) | Stay on CEF; no engine abstraction or WKWebView backend (codec/DRM gains do not outweigh losing request-path adblock, CDP, and Chromium compat); reopen via a spike if DRM becomes the main complaint | Superseded by [0072](0072-system-webviews-over-cef-gated-on-spike.md) |
| [0067](0067-little-windows-hold-normal-profile-tabs.md) | Little windows hold normal-profile tabs in a separate core list outside the sidebar; Cmd+O moves the live page into the main window; a favorite's off-site link opens one; links from other apps open one by default | Accepted |
| [0068](0068-overlay-state-machine-in-core.md) | The command bar and address edit are one pure core state machine: the destination is part of the state, a close only affects the surface it names, a stale commit is dropped; Swift renders the state | Accepted |
| [0069](0069-pages-and-find-join-the-surface-machine.md) | History / Passwords / Settings pages and the find bar join the core surface machine (`Ui { overlay, find, pages }`); a page closes input surfaces and find; Swift page flags are renderer-only | Accepted |
| [0070](0070-release-signing-without-restricted-entitlements.md) | Release builds sign without restricted entitlements unless a provisioning profile is embedded (guarded); hardened-runtime device entitlements added; CI requires Developer ID, notarization and a verified update signature | Accepted |
| [0071](0071-core-omnibox-ranker.md) | The omnibox ranks in the core on Chromium's relevance scale: only an input-led row can be the default (preselected, run by Return) and it stays stable; inline completion only for typed sites; learns from picks (Firefox adaptive history); open tabs switch | Accepted |
| [0072](0072-system-webviews-over-cef-gated-on-spike.md) | Move to system webviews (WKWebView first) behind a Rust engine trait to hand the security treadmill to the OS vendor; ad blocking as compiled content rules + page-world scriptlets + hostname proxy; CDP dropped; gated on a WKWebView spike (P0-5) | Accepted (supersedes ADR-0066) |
