# Parity Ledger

**Role:** the *coverage / conformance* view — the one place that enumerates every user-facing
capability and tracks, **per frontend**, whether it's built and whether it honors the
cross-platform contract. None of the other docs answer *"for feature X, which frontends have it,
and does each match?"* — the roadmap is temporal (what's next in time), the backlog is a work
queue, the spec/style-guide define behavior. This is the grid: **capability × frontend × status**.

Companion to two docs it deliberately does **not** duplicate:

- **`style-guide §2` — the consistency contract.** Defines the *rule*: what must be identical
  across platforms vs. what must be native to each. This ledger applies that rule row by row.
- **`spec §6` — Arc UX → native mapping.** Defines *how* each behavior maps onto native widgets
  (SwiftUI `List` ↔ WinUI `ItemsRepeater`, …). That's the design; this is the tracker of whether
  the design is realized.

Refs: `spec §N` = architecture.md · `style §N` = style-guide.md · `mvp` = mvp1.md ·
`rm §N` = roadmap.md · `ADR-NNNN` = adr/.

---

## How to read it

Every row is **classified** by the `style §2` split — this is the "consistent UI conforming to
platform idiosyncrasies" nuance made concrete:

- **Inv** (invariant) — must be **behaviorally identical** on every platform. Same model, same
  flow, same shortcut. A divergence here is a **bug**. Parity = "matches the others."
- **Idiom** — must **adapt** per platform (materials, controls, icons, motion curves, window
  conventions). Forcing uniformity here would be the bug. Parity = "the adaptation exists and
  follows the contract," *not* "looks identical." These rows are the intentional divergences —
  documented so nobody mistakes a deliberate platform difference for a gap and "fixes" it into
  sameness.

**Status:** `✅` done · `🚧` partial (functional but not final, or not yet verified on a running
app) · `⬜` not started · `➖` N/A by design.

Because ~80% of behavior lives in the shared Rust core, most **Inv** parity is guaranteed *by
construction* — if a feature lives in the core, every frontend gets identical behavior for free.
The real risk surface is the **native layer** (view + binding), so for a core-provided feature the
tracked cell means *"is the native surface wired,"* not *"was the logic reimplemented."* Longer term
the core-feature rows should be anchored to the FFI manifest (every command/event a frontend must
bind) so they can't silently drift from the code — see ADR-0007 · *Future*.

## How it stays honest

- **Status only.** Behavior is defined in `style`/`spec` (linked in the last column); this file
  never re-explains it. One source of truth per fact — same discipline as the backlog.
- **Flipping a cell is part of "done."** Landing a capability on a frontend means updating its cell
  here, exactly like marking a backlog item `done`.
- **A real gap on a shipping frontend becomes a backlog row.** A `⬜`/`🚧` that's on the critical
  path is filed as a `next` item in `backlog.md` — this ledger is the coverage map, the backlog is
  the queue.

> **Current state (2026-07-06):** one live frontend (macOS), so it's one mostly-populated column and
> two empty ones. That's the point *now*: it forces a full enumeration of the surface (so "done" on
> macOS is measured against the whole list, not vibes), and it is the literal build-spec for Windows
> and Linux when they start — each `⬜` is a to-do. **macOS statuses are the read against the working
> tree; several are build-verified, not yet running-app-verified** — those depend on the P0-1
> focus-routing proof (`backlog.md`) before they can be confirmed `✅`.

---

## Shell & layout

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Slim, content-first, full-bleed content (no Arc inset frame) | Inv | 🚧 | ⬜ | ⬜ | style §3 |
| Sidebar material (Liquid Glass / Mica / GTK-native) | Idiom | 🚧 | ⬜ | ⬜ | style §2,§3 · spec §7,§8 |
| Sidebar collapse/expand (spring width) | Inv | ⬜ | ⬜ | ⬜ | style §3,§9 |
| Toolbar controls + Arc placement (sidebar-toggle + back/fwd left, reload right) | Inv | 🚧 | ⬜ | ⬜ | style §3 · ADR-0004 |
| Window-control placement (traffic lights inset / caption buttons) | Idiom | 🚧 | ⬜ | ⬜ | style §3 |
| Sidebar URL chip (current-tab address; click / ⌘L → edit inline in the sidebar) | Inv | 🚧 | ⬜ | ⬜ | style §6 · ADR-0014 |

## Tabs

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Vertical tab list (favicon/title/close/select) | Inv | 🚧 | ⬜ | ⬜ | style §4 · spec §6 |
| Create / close / **reorder** / select motion | Inv | 🚧 | ⬜ | ⬜ | style §4,§7 |
| Drag: reorder inline · pin/unpin by drag · file into/out of a folder | Inv | 🚧 | ⬜ | ⬜ | style §4,§7 · ADR-0015 |
| Live state (title/favicon/spinner) | Inv | ✅ | ⬜ | ⬜ | style §4 · spec §5 |
| Audio-playing indicator + click-to-mute | Inv | ⬜ | ⬜ | ⬜ | style §4 |
| New tab adjacent + focused; close selects neighbor | Inv | 🚧 | ⬜ | ⬜ | style §4 |
| Reopen closed tab (⌘⇧T) — faithful restore of space/folder/pinned/slot | Inv | 🚧 | ⬜ | ⬜ | style §4 · ADR-0013 |
| Saved vs. open tab split | Inv | 🚧 | ⬜ | ⬜ | style §4 · ADR-0006 |
| Session persistence — tabs/spaces/folders survive restart (lazy hydration) | Inv | 🚧 | ⬜ | ⬜ | rm §4 · ADR-0013 |

## Spaces

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Space switcher (compact, in sidebar) | Idiom | 🚧 | ⬜ | ⬜ | style §5 · spec §6 |
| Space emoji icon + name (icon in core; switcher renders the icon) | Inv | 🚧 | ⬜ | ⬜ | style §5 |
| Per-space tab sets (switch swaps visible tabs) | Inv | 🚧 | ⬜ | ⬜ | style §5 |
| Create / switch / delete spaces | Inv | 🚧 | ⬜ | ⬜ | style §5 |
| Delete-with-tabs prompts / archives (never silent drop) | Inv | 🚧 | ⬜ | ⬜ | style §5 |

*macOS: spaces (P1-M4), folders/transforms (P1-7), and emoji icons (P1-12) are built; 🚧 pending the
focus-routing proof + polish pass that gates ✅ across the board.*

## Command bar

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Invocation: Cmd/Ctrl+T (new) · Cmd/Ctrl+L (edit-in-place) | Inv | 🚧 | ⬜ | ⬜ | style §6 · spec §9 |
| Unified ranked results (tabs · history · URL/search) | Inv | 🚧 | ⬜ | ⬜ | style §6 · spec §9 |
| Keyboard-driven end to end (arrows/Enter/Escape) | Inv | 🚧 | ⬜ | ⬜ | style §6 |
| Expand morph animation | Idiom | 🚧 | ⬜ | ⬜ | style §6 · spec §9 |
| Transient overlay (never a persistent full-width bar) | Inv | 🚧 | ⬜ | ⬜ | style §6,§13 |

## Navigation & history

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Back / forward / reload / hard-reload | Inv | ✅ | ⬜ | ⬜ | style §9 · spec §4 |
| Zoom | Inv | ✅ | ⬜ | ⬜ | mvp |
| Find in page (Cmd/Ctrl+F) | Inv | ✅ | ⬜ | ⬜ | style §9 |
| Address entry: command bar (new tab) + inline sidebar chip (current tab) | Inv | 🚧 | ⬜ | ⬜ | style §6,§13 · ADR-0014 |
| History fuzzy autocomplete (class-ranked + frecency) + match highlighting | Inv | ✅ | ⬜ | ⬜ | style §6 · spec §9 · ADR-0011 |
| Inline URL autocomplete (default-match completion, selected tail) | Inv | 🚧 | ⬜ | ⬜ | ADR-0011 |
| Favicon fetch + caching | Inv | ✅ | ⬜ | ⬜ | style §4 · mvp |
| History page (⌘Y surface: day-grouped list, live search, clear, row remove) | Inv | ✅ | ⬜ | ⬜ | style §15 · ADR-0010 |

## Keyboard & motion

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Keyboard shortcut table (Cmd↔Ctrl, same actions) | Inv | 🚧 | ⬜ | ⬜ | style §9 |
| Motion moments (create/close/reorder/select, sidebar, space, command-bar, hover/focus) | Inv | 🚧 | ⬜ | ⬜ | style §7 |
| Motion curves (native spring/easing) | Idiom | 🚧 | ⬜ | ⬜ | style §7 |
| Honor reduce-motion (cut/cross-fade instead of move) | Inv | ⬜ | ⬜ | ⬜ | style §7,§11 |

*macOS shortcuts: Cmd+T/S/R/[/]/W run through a Cmd-gated key monitor; the full `style §9` set and
focus routing (P0-1) are unproven.*

## Iconography & typography

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Native icon set, same *meaning* (SF Symbols / Segoe Fluent / platform set) | Idiom | ✅ | ⬜ | ⬜ | style §8 · spec §7,§8 |
| System UI font + type scale | Idiom | ✅ | ⬜ | ⬜ | style §8 |
| Respect OS text scaling / Dynamic Type | Inv | ⬜ | ⬜ | ⬜ | style §8,§11 |

## States & feedback

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Loading (per-tab spinner + progress affordance) | Inv | 🚧 | ⬜ | ⬜ | style §10 |
| Native error pages (ALLOY runtime) | Idiom | ⬜ | ⬜ | ⬜ | style §10 · spec §7 |
| Empty states (zero-tab, no history, empty results) | Inv | 🚧 | ⬜ | ⬜ | style §10 |
| Hover / focus / active / disabled on every control | Inv | 🚧 | ⬜ | ⬜ | style §10 |

*macOS: zero-tab empty state is `✅` (P1-6); the other empty/loading states are partial.*

## Accessibility

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Screen-reader via native controls (VoiceOver / Narrator) | Idiom | 🚧 | ⬜ | ⬜ | style §11 · spec §14 |
| Full keyboard navigation + always-visible focus | Inv | ⬜ | ⬜ | ⬜ | style §9,§11 |
| Reduce-transparency fallback (solid material off glass) | Inv | ⬜ | ⬜ | ⬜ | style §11 |
| Increased-contrast honored | Inv | ⬜ | ⬜ | ⬜ | style §11 |
| Accessible labels on icon-only controls | Inv | ⬜ | ⬜ | ⬜ | style §11 |

## Engine & feature surface

Core-provided; parity is largely by construction (shared core), so the cell tracks the **native
surface**. `⬜ (post-MVP)` = spec'd but out of MVP1 scope — listed so the surface is complete.

| Capability | Class | macOS | Win | Linux | Defined in |
|---|:---:|:---:|:---:|:---:|---|
| Passkeys / WebAuthn (needs signed bundle) | Inv | ⬜ | ⬜ | ⬜ | spec §6 · mvp |
| Adblock — request-path + cosmetic + UI toggle | Inv | 🚧 (network + shield UI + cosmetic element-hiding; procedural/generic/scriptlets/subframe follow-ups) | ⬜ | ⬜ | spec §11 · mvp · ADR-0020 |
| Cookie-consent (reject + hide, own toggle) | Inv | 🚧 (reject OneTrust/Cookiebot/Didomi + Tier-A hide, View-menu toggle; main + same-origin frames; full lists + more CMPs + TCF → P2-26; cross-origin iframes → P2-24/ADR-0027) | ⬜ | ⬜ | ADR-0026 · rm §3.1 |
| Appearance — OS color scheme → web content + Light/Dark/System (chrome + content) | Inv | ✅ (global request context; View → Appearance menu + `NSApp.appearance`; System tracks OS live) | ⬜ | ⬜ | ADR-0030 · style §18 |
| DevTools (popup; Cmd/Ctrl+Opt+I) | Inv | ✅ | ⬜ | ⬜ | spec §13 · style §9 |
| CDP automation surface (loopback, consent-gated) | Inv | ⬜ | ⬜ | ⬜ | spec §13 |
| Tweaks — Snip / Restyle / Script (picker · editor · manager · safety UX) | Inv | ⬜ | ⬜ | ⬜ | style §14 · spec §20 · ADR-0002 |
| Password vault UI (setup · unlock · list/search · reveal · add/edit/delete · import · sidebar access) | Inv | 🚧 (settings · registration pending) | ⬜ | ⬜ | spec §10 · ADR-0010/0016 |
| Password vault autofill | Inv | 🚧 (substrate + in-page multi-match picker + URL-bar quick-access button [Fill/Copy]; SPA/two-step detection → P2-28/29, native save banner → P2-23/M2) | ⬜ | ⬜ | spec §10 · ADR-0016/0017/0029 |
| Sync | Inv | ⬜ (post-MVP) | ⬜ | ⬜ | spec §12 |
| Downloads UI (beyond command-bar suggestions) | Inv | ⬜ (post-MVP) | ⬜ | ⬜ | mvp (out) |
| Settings UI (beyond minimum) | Inv | ⬜ (post-MVP) | ⬜ | ⬜ | mvp (out) |
| Agent layer (ACP / tools) | Inv | ⬜ (post-MVP) | ⬜ | ⬜ | spec §19 |

---

*Seeded 2026-07-06 from `style-guide.md`, `architecture.md`, `mvp1.md`, and the backlog's working-tree
reconciliation. macOS is the only live frontend; Windows/Linux columns are the forward build-spec.
Linux toolkit is not yet locked (`spec §8` leaves it open) — its idiom rows resolve when it's chosen.
Decision & rationale: ADR-0007.*
