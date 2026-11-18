# Parity Ledger

**Role:** the *coverage / conformance* view - the one place that enumerates every user-facing
capability and tracks, **per frontend**, whether it's built and whether it honors the
cross-platform contract. None of the other docs answer *"for feature X, which frontends have it,
and does each match?"* - the roadmap is temporal (what's next in time), the backlog is a work
queue, the spec/style-guide define behavior. This is the grid: **capability × frontend × status**.

Companion to two docs it deliberately does **not** duplicate:

- **`style-guide §2` - the consistency contract.** Defines the *rule*: what must be identical
  across platforms vs. what must be native to each. This ledger applies that rule row by row.
- **`spec §6` - Arc UX → native mapping.** Defines *how* each behavior maps onto native widgets
  (SwiftUI `List` ↔ WinUI `ItemsRepeater`, …). That's the design; this is the tracker of whether
  the design is realized.

Refs: `spec §N` = architecture.md · `style §N` = style-guide.md · `mvp` = mvp1.md ·
`rm §N` = roadmap.md · `ADR-NNNN` = adr/.

---

## How to read it

Every row is **classified** by the `style §2` split - this is the "consistent UI conforming to
platform idiosyncrasies" nuance made concrete:

- **Inv** (invariant) - must be **behaviorally identical** on every platform. Same model, same
  flow, same shortcut. A divergence here is a **bug**. Parity = "matches the others."
- **Idiom** - must **adapt** per platform (materials, controls, icons, motion curves, window
  conventions). Forcing uniformity here would be the bug. Parity = "the adaptation exists and
  follows the contract," *not* "looks identical." These rows are the intentional divergences -
  documented so nobody mistakes a deliberate platform difference for a gap and "fixes" it into
  sameness.

**Status:** `✅` done · `🚧` partial (functional but not final, or a shipped core with named
follow-ups) · `⬜` not started · `➖` N/A by design.

Because ~80% of behavior lives in the shared Rust core, most **Inv** parity is guaranteed *by
construction* - if a feature lives in the core, every frontend gets identical behavior for free.
The real risk surface is the **native layer** (view + binding), so for a core-provided feature the
tracked cell means *"is the native surface wired,"* not *"was the logic reimplemented."* Longer term
the core-feature rows should be anchored to the FFI manifest (`crates/ffi/src/lib.rs` - every
command/event a frontend must bind) so they can't silently drift from the code - see ADR-0007 ·
*Future*.

## How it stays honest

- **Status only.** Behavior is defined in `style`/`spec` (linked in the last column); this file
  never re-explains it. One source of truth per fact - same discipline as the backlog.
- **Flipping a cell is part of "done."** Landing a capability on a frontend means updating its cell
  here, exactly like marking a backlog item `done`.
- **A real gap on a shipping frontend becomes a backlog row.** A `⬜`/`🚧` that's on the critical
  path is filed as a `next` item in `backlog.md` - this ledger is the coverage map, the backlog is
  the queue.

> **Current state (2026-07-13):** one live frontend (macOS) plus **Windows now starting as the
> active second frontend.** macOS **MVP1 (M1–M5) is complete and daily-driven**, and the P0-1
> CEF↔native focus-routing proof has landed (`just focus-selftest`, ADR-0018) - so the macOS cells
> below are the read against the **running app**, not build-only, and the earlier "🚧 until focus
> routing proves out" caveat is retired (rows that are still `🚧` are genuine partials, noted
> inline). The forward purpose stands: the full enumeration keeps "done on macOS" measured against
> the whole surface, and it is the **literal build-spec for Windows** - every `⬜` in the Win column
> is a to-do - and later Linux.

---

## Shell & layout

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Slim, content-first, full-bleed content (no Arc inset frame) | Inv | ✅ | ⬜ | ⬜ | style §3 |
| Sidebar material (Liquid Glass / Mica / GTK-native) | Idiom | ✅ | ⬜ | ⬜ | style §2,§3 · spec §7,§8 · ADR-0019 |
| Sidebar collapse/expand (spring width) | Inv | ✅ | ⬜ | ⬜ | style §3,§9 |
| Toolbar controls + Arc placement (sidebar-toggle + back/fwd left, reload right) | Inv | ✅ | ⬜ | ⬜ | style §3 · ADR-0004 |
| Window-control placement (traffic lights inset / caption buttons) | Idiom | ✅ | ⬜ | ⬜ | style §3 · ADR-0012 |
| Sidebar URL chip (current-tab address; click / ⌘L → edit inline in the sidebar) | Inv | ✅ | ⬜ | ⬜ | style §6 · ADR-0014 |
| Viewport address bar - page theme-color chrome (always when in-page mode; scroll-gated matched fill) | Idiom | 🚧 (viewport bar landed; interaction polish + theme-color chrome → P1-28) | ⬜ | ⬜ | P1-28 · `theme_color.rs` |
| Layout setting: Sidebar \| Classic (global, core-stored) | Inv | 🚧 (core + FFI landed P2-56; picker in P2-57) | ⬜ | ⬜ | style §3 · ADR-0073 |
| Classic compact row: active tab is the address field; Space picker; icon tabs; folder menus; overflow scroll | Inv | ⬜ (P2-57..P2-60) | ⬜ | ⬜ | style §3 · ADR-0073 |
| Sidebar page-aware tint (optional setting; per-tab color, no scroll gating) | Idiom | ✅ (Settings → Appearance → Match page color; off by default) | ⬜ | ⬜ | P2-37 · `theme_color.rs` |

## Tabs

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Vertical tab list (favicon/title/close/select) | Inv | ✅ | ⬜ | ⬜ | style §4 · spec §6 |
| Create / close / **reorder** / select motion | Inv | ✅ | ⬜ | ⬜ | style §4,§7 |
| Drag: reorder inline · pin/unpin by drag · file into/out of a folder | Inv | ✅ | ⬜ | ⬜ | style §4,§7 · ADR-0015 · ADR-0035 · `just drag-selftest` |
| Live state (title/favicon/spinner) | Inv | ✅ | ⬜ | ⬜ | style §4 · spec §5 |
| Audio-playing indicator + click-to-mute | Inv | ⬜ | ⬜ | ⬜ | style §4 |
| New tab adjacent + focused; close selects neighbor | Inv | ✅ | ⬜ | ⬜ | style §4 |
| Reopen closed tab (⌘⇧T) - faithful restore of space/folder/pinned/slot | Inv | ✅ | ⬜ | ⬜ | style §4 · ADR-0013 |
| Pinned vs. Open tab split (+ multiselect, folder-from-selection, clear Open) | Inv | ✅ | ⬜ | ⬜ | style §4 · ADR-0006,0057 |
| Arc Pin/Favorite terminology; closing any Pinned tab preserves it; delete is explicit | Inv | ✅ | ⬜ | ⬜ | ADR-0057 |
| Per-Space Pinned section collapse persisted; collapsed section peeks active pin and accepts drops | Inv | ✅ | ⬜ | ⬜ | ADR-0057 |
| Folder expand/collapse persisted (survives space switch); collapsed folder peeks the active tab | Inv | ✅ | ⬜ | ⬜ | ADR-0039 |
| Pinned/Favorite URL identity - navigation is temporary; close returns home; only explicit replace moves the pin | Inv | ✅ | ⬜ | ⬜ | style §4 · ADR-0056,0057 |
| Favorites - permanent Arc-style icon strip; click focuses, ⌘W resets+deselects (icon never leaves); per-Space active state; glass means selected (idle is a quiet fill) | Inv | ✅ | ⬜ | ⬜ | style §4 · ADR-0032,0038,0044,0046,0056,0057,0059 |
| Runtime tab suspension - auto-discard background tabs (favorites resident); suspended rows render identically | Inv | ✅ | ⬜ | ⬜ | ADR-0033,0045 |
| Session persistence - tabs/spaces/folders survive restart (lazy hydration) | Inv | ✅ | ⬜ | ⬜ | rm §4 · ADR-0013 |

## Spaces

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Space switcher (compact, in sidebar) | Idiom | ✅ | ⬜ | ⬜ | style §5 · spec §6 |
| Space emoji icon + name (icon in core; switcher renders the icon) | Inv | ✅ | ⬜ | ⬜ | style §5 |
| Per-space tab sets (switch swaps visible tabs); filmstrip slide on switch | Inv | ✅ | ⬜ | ⬜ | style §5 · ADR-0043 |
| Create / switch / delete / rename spaces; swipe-to-switch | Inv | ✅ | ⬜ | ⬜ | style §5 |
| Folder↔space transform (bidirectional) | Inv | ✅ | ⬜ | ⬜ | ADR-0006 |
| Delete-with-tabs prompts / archives (never silent drop) | Inv | ✅ | ⬜ | ⬜ | style §5 |

## Command bar

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Invocation: Cmd/Ctrl+T (new) · Cmd/Ctrl+L (edit-in-place, inline chip) | Inv | ✅ | ⬜ | ⬜ | style §6 · spec §9 · ADR-0014 |
| Unified ranked results (tabs · history · URL/search) | Inv | ✅ | ⬜ | ⬜ | style §6 · spec §9 |
| Built-in browser commands + named Space switching in Cmd/Ctrl+T | Inv | ✅ | ⬜ | ⬜ | ADR-0034,0058 |
| Keyboard-driven end to end (arrows/Enter/Escape) | Inv | ✅ | ⬜ | ⬜ | style §6 |
| Expand morph animation | Idiom | 🚧 (glassEffectID morph landed; feel-tuning pass pending) | ⬜ | ⬜ | style §6 · spec §9 |
| Transient overlay (never a persistent full-width bar) | Inv | ✅ | ⬜ | ⬜ | style §6,§13 |

## Navigation & history

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Back / forward / reload / hard-reload | Inv | ✅ | ⬜ | ⬜ | style §9 · spec §4 |
| Zoom | Inv | ✅ | ⬜ | ⬜ | mvp |
| Find in page (Cmd/Ctrl+F) | Inv | ✅ | ⬜ | ⬜ | style §9 |
| Address entry: command bar (new tab) + inline sidebar chip (current tab) | Inv | ✅ | ⬜ | ⬜ | style §6,§13 · ADR-0014 |
| History fuzzy autocomplete (class-ranked + frecency) + match highlighting | Inv | ✅ | ⬜ | ⬜ | style §6 · spec §9 · ADR-0011 |
| Inline URL autocomplete (default-match completion, selected tail) | Inv | ✅ | ⬜ | ⬜ | ADR-0011 |
| Favicon fetch + caching | Inv | ✅ | ⬜ | ⬜ | style §4 · mvp |
| History page (⌘Y surface: day-grouped list, live search, clear, row remove) | Inv | ✅ | ⬜ | ⬜ | style §15 · ADR-0010 |

## Keyboard & motion

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Keyboard shortcut table (Cmd↔Ctrl, same actions) | Inv | ✅ | ⬜ | ⬜ | style §9 |
| Motion moments (create/close/reorder/select, sidebar, space, command-bar, hover/focus) | Inv | ✅ | ⬜ | ⬜ | style §7 · ADR-0041,0043 |
| Motion curves (native spring/easing) | Idiom | ✅ | ⬜ | ⬜ | style §7 |
| Honor reduce-motion (cut/cross-fade instead of move) | Inv | 🚧 (honored in the built animations; not yet a full-surface audit) | ⬜ | ⬜ | style §7,§11 |

*macOS shortcuts: the full `style §9` table is wired through `NSApp.mainMenu` (P1-M5); focus routing
is proven by `just focus-selftest` (P0-1 / ADR-0018).*

## Iconography & typography

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Native icon set, same *meaning* (SF Symbols / Segoe Fluent / platform set) | Idiom | ✅ | ⬜ | ⬜ | style §8 · spec §7,§8 |
| System UI font + type scale | Idiom | ✅ | ⬜ | ⬜ | style §8 |
| Respect OS text scaling / Dynamic Type | Inv | ⬜ | ⬜ | ⬜ | style §8,§11 |

## States & feedback

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Loading (per-tab spinner + progress affordance) | Inv | 🚧 (spinner done; richer progress affordance partial) | ⬜ | ⬜ | style §10 |
| Native error pages (ALLOY runtime) | Idiom | ⬜ (currently defers to Chromium's net-error page) | ⬜ | ⬜ | style §10 · spec §7 |
| Empty states (zero-tab, private-window, no history, empty results) | Inv | 🚧 (zero-tab + private-window done; history/results partial) | ⬜ | ⬜ | style §10 |
| Hover / focus / active / disabled on every control | Inv | ✅ (shared `IconButtonStyle`) | ⬜ | ⬜ | style §10,§17 · ADR-0023 |

## Accessibility

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Screen-reader via native controls (VoiceOver / Narrator) | Idiom | 🚧 (free via native controls; custom-drawn traffic lights need labels - P2-7) | ⬜ | ⬜ | style §11 · spec §14 |
| Full keyboard navigation + always-visible focus | Inv | ⬜ | ⬜ | ⬜ | style §9,§11 |
| Reduce-transparency fallback (solid material off glass) | Inv | ⬜ | ⬜ | ⬜ | style §11 |
| Increased-contrast honored | Inv | ⬜ | ⬜ | ⬜ | style §11 |
| Accessible labels on icon-only controls | Inv | ⬜ | ⬜ | ⬜ | style §11 |

## Multi-window, privacy & switching

Tier-0/Tier-1 surfaces (`rm §4`) beyond the single-window Arc shell - each a distinct native
window/lifecycle surface a Windows build must reproduce.

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Multiple browser windows (independent, over the shared core) | Inv | ✅ | ⬜ | ⬜ | ADR-0031 |
| Private / incognito windows (⌘⇧N; incognito `RequestContext`, ephemeral tabs, privacy gates, opt-in vault fill) | Inv | ✅ | ⬜ | ⬜ | ADR-0031 |
| Little windows (⌥⌘N; one normal-profile tab outside the sidebar; ⌘O moves the live page into the main window; links from other apps and a favorite's off-site links open one) | Inv | ✅ | ⬜ | ⬜ | ADR-0067 |
| Set as default browser + open links from other apps | Idiom | ✅ | ⬜ | ⬜ | rm §4.1 · `DefaultBrowser.swift` |
| Import from Chrome / Safari / Arc (spaces/folders/tabs; passwords via vault import) | Inv | 🚧 (Arc sidebar export in Settings; Chrome/Safari extraction → P2-18) | ⬜ | ⬜ | rm §5.2 · `ArcImport.swift` |

## Engine & feature surface

Core-provided; parity is largely by construction (shared core), so the cell tracks the **native
surface**. `⬜ (post-MVP)` = spec'd but out of MVP1 scope - listed so the surface is complete.

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Passkeys / WebAuthn (OS platform authenticator / iCloud Keychain) | Inv | ⬜ (unsigned builds disable the API, CRASH-0003; P0-2 signed+entitled) | ⬜ | ⬜ | spec §6 · mvp · ADR-0061 |
| Proprietary codecs (H.264 / AAC MP4 playback) - custom CEF build | Inv | ✅ (macOS-arm64 codec build) | ⬜ (needs its own codec build, else codec-less fallback) | ⬜ | ADR-0040,0042 · rm §3.2 · `scripts/build-cef-codecs.sh` |
| HTML5 / video content fullscreen (OS fullscreen + full-bleed chrome hide) | Inv | ✅ (Alloy `OnFullscreenModeChange` → macOS `toggleFullScreen` + chrome hide; exit path) | ⬜ | ⬜ | P2-48 · CEF Alloy host responsibility |
| Adblock - request-path + cosmetic + UI toggle | Inv | 🚧 (network + shield UI + cosmetic element-hiding done; procedural/generic/scriptlets/subframe follow-ups) | ⬜ | ⬜ | spec §11 · mvp · ADR-0020 |
| Cookie-consent (reject + hide, own toggle) | Inv | 🚧 (reject OneTrust/Cookiebot/Didomi + Tier-A hide + View-menu toggle, main + same-origin frames; full lists/more CMPs/TCF → P2-26; cross-origin iframes → P2-24/ADR-0027) | ⬜ | ⬜ | ADR-0026 · rm §3.1 |
| Appearance - OS color scheme → web content + Light/Dark/System (chrome + content) | Inv | ✅ (global request context; View → Appearance menu + `NSApp.appearance`; System tracks OS live) | ⬜ | ⬜ | ADR-0030 · style §18 |
| DevTools (popup; Cmd/Ctrl+Opt+I) | Inv | ✅ | ⬜ | ⬜ | spec §13 · style §9 · ADR-0021 |
| CDP automation surface (loopback, consent-gated) | Inv | ✅ (off by default; `browser_cdp_set_enabled`) | ⬜ | ⬜ | spec §13 · rm §3.5 |
| Content-script injection substrate (document-start, isolated world) | Inv | ✅ (autofill + cosmetic + consent consumers) | ⬜ | ⬜ | ADR-0017 · spec §11,§20 |
| Tweaks - Snip / Restyle / Script (picker · editor · manager · safety UX) | Inv | ⬜ | ⬜ | ⬜ | style §14 · spec §20 · ADR-0002 |
| Commands - user scripts (command bar · palette · settings manager · cookbook) | Inv | ⬜ | ⬜ | ⬜ | ADR-0034 · rm §5.3 |
| Commands - agentic create/edit (in-app AI authoring) | Idiom | ⬜ | ⬜ | ⬜ | ADR-0034 · spec §19 · rm §5.4 · P4-1 |
| Password vault UI (setup · unlock · list/search · reveal · add/edit/delete · import · sidebar access) | Inv | 🚧 (settings · registration pending) | ⬜ | ⬜ | spec §10 · ADR-0010/0016 |
| Password vault autofill | Inv | 🚧 (substrate + in-page multi-match picker + URL-bar quick-access [Fill/Copy] + core-initiated fill + last-used ordering + whole-vault search; SPA/two-step detection → P2-28/29, native save banner → P2-23/M2) | ⬜ | ⬜ | spec §10 · ADR-0016/0017/0029/0048/0049 |
| Settings UI (beyond minimum) | Inv | 🚧 (Appearance, page tint, tab-suspension, Arc import, vault; growing) | ⬜ | ⬜ | mvp · `SettingsView.swift` |
| Sync | Inv | ⬜ (post-MVP) | ⬜ | ⬜ | spec §12 |
| Downloads UI (beyond command-bar suggestions) | Inv | ⬜ (post-MVP) | ⬜ | ⬜ | mvp (out) |
| Auto-update (check · signed download · Update & Restart) | Inv | ✅ (Zinnia model: GitHub + minisign; sidebar card; Sparkle later) | ⬜ | ⬜ | ADR-0052 · rm §6.2 · P2-47 |
| Agent layer (ACP / tools) | Inv | ⬜ (post-MVP) | ⬜ | ⬜ | spec §19 |

---

*Seeded 2026-07-06 from `style-guide.md`, `architecture.md`, `mvp1.md`, and the backlog's working-tree
reconciliation; **reconciled 2026-07-13 against the running macOS build for the Windows kickoff**
(MVP1 complete + ADRs 0031–0046 landed). macOS is the primary live frontend; the Windows column is
the active forward build-spec, Linux the later one. Linux toolkit is not yet locked (`spec §8` leaves
it open) - its idiom rows resolve when it's chosen. Decision & rationale: ADR-0007.*
