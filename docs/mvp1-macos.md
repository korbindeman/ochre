# MVP 1 — macOS, Arc UX

**Goal:** a daily-drivable macOS browser with Arc's core interaction model and native Liquid Glass chrome. One platform, no sync, no agent layer, no vault. Proves the whole thesis — native chrome + Arc UX + Chromium — on macOS.

**Precondition:** the step-0 spike passes — single CEF tab in a native `NSView`, DevTools opens, a passkey login works, `adblock` blocks on the request path. MVP builds directly on that foundation.

Companion to the full architecture spec; section refs (§) point there.

---

## Scope

**In:**
- Multi-tab CEF — one browser per tab, hosted in `NSView`s (windowed rendering, §4).
- Arc vertical-tab sidebar: create / close / reorder / select, with live title / favicon / load state.
- Cmd+T center command bar with the Liquid Glass morph (open URL / search / switch to an open tab).
- Native Liquid Glass chrome (glass sidebar + minimal toolbar), SF Symbols.
- Core navigation: back / forward / reload; address entry via the command bar.
- Adblock (`adblock` crate on the CEF request path, §11) — cheap once the path exists.
- Spaces: switchable tab sets (create / switch / delete), each owns its tabs. **No** per-space profile / theme / color.

**Out (named so they don't creep in):**
- Windows (→ MVP 2).
- Sync (§12), agent layer / ACP / tools (§19), password vault (§10).
- Extensions (dropped by design), DRM/Widevine (build-flag; enable only if you actually need protected video).
- Advanced Arc: split view, easels, boosts, peek, Little Arc, tab auto-archive, per-space profiles/themes/colors.
- Settings UI beyond the minimum; history/downloads UI beyond command-bar suggestions.
- Docked DevTools polish — MVP uses CEF `ShowDevTools` in a popup (§13).

**Spaces:** firm MVP feature, stripped to essentials — switchable tab sets only, no per-space profile / theme / color. The tab model is **space-aware from M1** (a tab belongs to a space) rather than bolting spaces on later.

---

## Visual style

**macOS-native, Safari-slim, content-first.** Arc's *interaction* model, not Arc's *chrome*.

- **Content comes first.** The web view is the priority and takes maximum real estate. **Remove Arc's thick inset frame** — no rounded card floated in from the edges with big margins. That specific look is the thing being killed.
- **Slim chrome, Safari-like.** A thin glass sidebar (vertical tabs + spaces) and minimal top controls (window controls + slim back/forward/reload). No persistent full-width address bar — URL entry is the Cmd+T command bar. Chrome stays out of the content's way.
- **Liquid Glass on the slim chrome.** Sidebar and any top strip are `NSGlassEffectView`; glass belongs on this top functional layer, which is exactly the slim floating chrome.
- **Target layout (native macOS 26 Safari look):** content extends full-bleed with the slim glass chrome floating *over* it, so the glass lenses live page content. **Validate in M2:** floating a glass view over the CEF `NSView` is the native-view-layering seam that can bite. **Fallback if troublesome:** content flush *beside* a slim glass sidebar — still slim + content-first, just no over-content lensing.
- **Icons:** SF Symbols throughout.
- **Sidebar collapse** lets content fill the whole window on demand.

---

## Arc UX — the minimal recognizable set

The features that make it read as "Arc," nothing more:
- Vertical tabs in a slim glass sidebar; spring animations on create/close/reorder/select.
- Spaces: a compact switcher in the sidebar; each space is its own tab set.
- Cmd+T command bar, centered, Spotlight-style glass expand.
- Sidebar collapse/expand toggle (⌘S-style) with a spring width animation.
- Pinned vs. ephemeral tabs — **simplified to one flat list** for MVP; the pinned/today split is a fast-follow.

---

## Build milestones (post-spike)

### M1 — Core tab model + multi-tab CEF (Rust)
`crates/core::tabs`: **space-aware** — a tab belongs to a space; create / close / select / reorder tabs, create / switch / delete spaces. Each tab = a CEF browser owning an `NSView` handle. Event stream: `TabTitleChanged`, `TabFaviconChanged`, `LoadStateChanged`, `NavigationCommitted`, `SpaceSwitched`. `crates/ffi` exposes commands + the event stream to Swift. Verify against a throwaway debug list before any real UI.

**Decision to lock here:** the observation pattern across the FFI. The core *pushes* events; Swift consumes them as an `AsyncStream`. Use **swift-bridge** (or a UniFFI callback interface) for this live, object-y boundary — plain request/response UniFFI is a poor fit for a stream of tab updates.

### M2 — SwiftUI shell + vertical-tab sidebar
Glass sidebar (`NSGlassEffectView` / `.glassEffect`); vertical tab list bound to core state; create/close/reorder/select with `.animation(.spring)` and `matchedGeometryEffect`; favicons / titles / spinners. CEF view hosted via `NSViewRepresentable` in the content region. Minimal toolbar (back / forward / reload, SF Symbols). **This is the heart of the MVP.**

**Highest-risk item:** keyboard/focus routing between the CEF `NSView` and SwiftUI — prove it here (typing in a page vs. Cmd+T vs. sidebar focus, and click-to-focus transitions).

### M3 — Cmd+T command bar
Centered `GlassEffectContainer` with the interactive glass morph (`glassEffectID` transition); text field + results (open tabs + history + URL/search); keyboard-driven; opens/switches tabs. Queries `command_bar_query` in the core. **The signature interaction — budget hand-tuning time; the glass morph APIs are new and agent training data is thin.**

### M4 — Spaces switcher + multi-space UX
Compact space switcher in the sidebar (icon row / swipe); create / switch / delete spaces; the sidebar shows the current space's tabs. The model is already space-aware from M1, so this milestone is the switching UX + animations + persistence. No per-space profile/theme/color.

### M5 — Adblock + polish
`adblock` crate on the CEF resource-request path (network + cosmetic). Sidebar collapse/expand; keyboard shortcuts (⌘T / ⌘W / ⌘⇧[ ] / ⌘‹number›); favicon caching; animation polish.

---

## Definition of done

- Renders and navigates real sites; links open in tabs; passkey logins work.
- Vertical-tab slim glass sidebar with Arc-like create/close/reorder/select — snappy.
- Spaces: switch between multiple tab sets; each keeps its own tabs.
- Cmd+T command bar with the Liquid Glass morph; opens URLs/searches, switches tabs.
- Slim, content-first, Safari-like chrome — no Arc inset frame; content maximized. Native Liquid Glass + SF Symbols; adblock active.
- Feels native and fast enough to daily-drive on macOS — the entire point of the project.

---

## MVP-specific risks

1. **CEF `NSView` ↔ SwiftUI focus/keyboard routing** (M2) — the seam most likely to bite; prove early.
2. **Command-bar glass morph fidelity** (M3) — new APIs, sparse examples; expect manual tuning.
3. **Per-tab CEF memory/lifecycle** with many tabs — set a warm/discard policy before tab counts get real.
4. **FFI event-stream ergonomics** (M1) — keeping SwiftUI in lockstep with core tab state without jank.

---

## Not in this doc

Full architecture, Windows, vault, sync, agent layer, DevTools/automation surface → the main architecture spec.
