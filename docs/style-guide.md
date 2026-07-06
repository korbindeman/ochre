# Style Guide

The rules for how this browser looks and behaves, so UI work stays consistent and production-grade without re-deciding each surface. Prescriptive on purpose — when a choice isn't covered here, default to what mainstream browsers (Safari, Chrome, Arc) already do, and add the rule here once made.

Companion to `architecture.md` and `mvp1.md`. This doc governs *look and behavior*; the spec governs *structure*. Cross-refs: `spec §N` → architecture.md, `mvp` → mvp1.md, bare `§N` → this doc.

---

## 1. Philosophy

- **Native materials, Arc interaction model.** The chrome is real platform glass/controls/icons (spec §1, §7, §8); the *interaction model* — vertical tabs, spaces, the center command bar — is Arc's, reproduced natively. We copy Arc's behavior, not Arc's rendered chrome.
- **Content first.** The web view is the product. Chrome is slim, floats out of the way, and yields maximum real estate to content. See §3.
- **Keyboard-first, built for power users.** Every action has a keyboard path; the command bar is the hub. The mouse is supported, never required. Defaults favor density and speed over hand-holding.
- **Production-grade always.** Every shipped surface is finished — real loading/error/empty states, correct focus and hover, no placeholders. If it can't be made production-quality, flag it rather than ship a half-version.
- **One behavior, native skin.** The UX is identical across platforms; the materials are native to each. This is the consistency contract (§2) — the single most important rule in this doc.

---

## 2. The consistency contract

"Platform-native UI, consistent UX" resolves to a hard split. Get this split right and the browser feels like one product with three native skins; get it wrong and it feels like three different browsers.

**Identical on every platform** (lives in the core, or is a cross-platform UX rule):

- Interaction model and information architecture — sidebar with vertical tabs + spaces, content region, center command bar. Same layout logic everywhere.
- Tab and space semantics — what a tab/space *is*, ordering, lifecycle, what belongs to what. Owned by the core (spec §4); the UI only renders it.
- Command-bar behavior — invocation, unified results, keyboard flow, dismissal (§6).
- Keyboard shortcuts — the same actions on the same keys (Cmd on macOS ↔ Ctrl on Windows/Linux); see §9.
- Terminology — one vocabulary across every platform and the code (§12).
- Motion intent — the same things animate at the same moments (create/close/reorder/select, sidebar toggle, command-bar expand). See §7.

**Native to each platform** (never forced into cross-platform uniformity):

- Materials — Liquid Glass on macOS, Mica/Acrylic on Windows, the platform-native equivalent on Linux (likely GTK4/libadwaita; not yet decided — spec doesn't lock it).
- Controls — buttons, inputs, sliders, menus, scrollbars are the platform's own, inheriting system look, behavior, and accessibility for free (spec §14).
- Icons — SF Symbols (macOS), Segoe Fluent Icons (Windows), the platform icon set on Linux. Same *meaning* (§8), native glyphs.
- Motion curves — native spring/easing systems (Core Animation, Composition, …). Same moments animate; the exact curve is the platform's.
- Window & system conventions — traffic lights vs. caption buttons, menu-bar vs. in-window menus, right-click menus, drag behavior. Follow the host OS.

**Rule of thumb:** if a user would expect it to match the rest of the OS, it's native. If a user would expect it to match the rest of *this browser*, it's identical. Behavior and model are ours; chrome and controls are the platform's.

**Tracked in `parity.md`** — this contract is applied row by row in the parity ledger, which records every capability's status per frontend and classes each as *Invariant* (must match) or *Idiom* (must adapt). That's where cross-frontend gaps live; this section is the rule it enforces.

---

## 3. Layout & structure

Slim, content-first, Safari-like. Arc's interaction model, **not** Arc's thick inset frame (`mvp1.md` "Visual style").

- **Content is full-bleed.** The web view takes the maximum area. No rounded card floated in with big margins, no persistent full-width address bar. That Arc look is explicitly killed.
- **Chrome is a slim glass sidebar + minimal top controls.** Sidebar holds the controls (sidebar toggle, back/forward/reload), a compact current-tab URL chip (§6 — edits inline, not a full-width bar), vertical tabs, and the space switcher. No persistent chrome beyond that — *new-tab* entry is the command bar (§6); editing the current tab's address happens inline in the chip.
- **Control placement mirrors Arc.** One top row shared with the traffic lights: lights, then the sidebar toggle beside them, back/forward/reload grouped at the right edge. The URL chip sits directly below. The New Tab row heads the today section of the tab list (below the pinned divider, above today's tabs) — it is not a bottom-of-sidebar control.
- **Sidebar sizing.** Comfortable default width, favicon + title per tab, no wasted padding. Draggable within a sane min/max. Collapsible to give content the whole window (§9 shortcut); collapse/expand is a spring width animation.
- **Glass floats over content where the platform allows it** (macOS 26 Safari target: glass lensing live content), with the fallback of content flush *beside* the sidebar if over-content compositing is troublesome (validate per `mvp1.md` M2). Either way: slim and content-first.
- **Glass belongs on the top functional layer only** (spec §7) — the slim floating chrome — never on content rows or as a stacked backdrop.
- **Traffic lights are custom-drawn in the sidebar; the real window buttons are hidden** (Arc-style, ADR-0012). The sidebar's top row leads with three lights we render ourselves as sidebar *content* — so they ride the sidebar's slide/fade animations natively and vanish with it, never stranding window controls over full-bleed content. They match native: exact colors, the window-inactive gray state, and hover-revealed × / − / fullscreen glyphs; clicks route to close / minimize / fullscreen. Fullscreen defers to the OS auto-hiding titlebar (the custom set steps aside, the real buttons come back). A deliberate, documented deviation from strict host-OS window-control placement (§2), chosen after three attempts at positioning/hiding the *real* buttons in sync with the sidebar drifted — AppKit re-lays them out on more triggers than any callback set covers, worst on the beta OS (macOS `TrafficLights.swift`).

---

## 4. Vertical tabs

The core of the Arc model. The sidebar is a vertical list; the core owns the model, the UI renders and animates it.

- **Row content:** favicon (or spinner while loading), title, and a close affordance on hover/selection. Truncate titles with an ellipsis; never wrap or reflow the row.
- **Selection:** one active tab per space, clearly indicated with a native selection treatment (not a custom-painted highlight that fights the platform).
- **Create / close / reorder / select** are all animated with native springs — snappy, never sluggish, never blocking interaction. Reorder is drag-and-drop *and* keyboard-driven (§9).
- **Live state** reflects the core event stream (spec §5): title, favicon, load spinner, and an audio-playing indicator with click-to-mute. Update without jank.
- **Pinned vs. ephemeral** tabs are a fast-follow — MVP is one flat list (`mvp1.md`). When added, pinned tabs sort above ephemeral and persist across sessions; the split must not change the rest of the model.
- **New tabs** open adjacent to the active tab and take focus, matching mainstream behavior. Closing the active tab selects a sensible neighbor (next, else previous).

---

## 5. Spaces

Switchable tab sets — each space owns its tabs (`mvp1.md`). The tab model is space-aware in the core; the UI is the switcher plus per-space rendering.

- **Switcher** is compact, lives in the sidebar (icon row / swipe), and shows which space is active. Switching is animated and instant-feeling; the sidebar re-renders the new space's tabs.
- **Spaces are identified by an emoji icon + a name.** The icon lives in the core model (one grapheme; core-owned like the rest of the structure) and is what the switcher renders — the name appears as a tooltip and in management dialogs. The default space is 🏠 "Home"; new spaces pick an icon at creation and can change it from the switcher context menu.
- **Each space is an independent tab set** — switching spaces swaps the visible tabs, not the window. Creating/switching/deleting spaces is keyboard- and pointer-driven.
- **No per-space profile / theme / color** in MVP — deliberately out (`mvp1.md`). Don't add per-space theming without a decision; it changes the model.
- Deleting a space with tabs prompts or safely archives — never silently drop a user's tabs.

---

## 6. Command bar (Cmd+T)

The signature interaction (spec §9). A center overlay, Spotlight-positioned, that expands with the native glass morph.

- **Invocation:** `Cmd/Ctrl+T` opens it for a new tab. (`Cmd/Ctrl+L` no longer opens the command bar — it edits the current tab's URL *inline in the sidebar chip*; see below and ADR-0014.) Text field auto-focused.
- **Unified results:** open tabs, history, and URL/search suggestions in one list, ranked, from the core's `command_bar_query` (spec §5). Typing filters live; the top result is actionable with Enter.
- **Keyboard-driven end to end:** arrow keys move selection, Enter commits, Escape (or click-away) dismisses. A power user never touches the mouse here.
- **Expand animation is the platform's morph** — Spotlight-style on macOS (`glassEffectID`), `ConnectedAnimation`/Composition on Windows. Budget hand-tuning; the feel is the point.
- **The command bar itself is never a persistent bar.** It's transient — appears on invocation, dismisses on commit or escape. There is no always-visible *full-width* address bar (§3, §13). It is the **new-tab** surface only; editing the current tab's address is inline (below), not a command-bar mode.
- **The sidebar's URL chip edits inline** (Arc, ADR-0014). Below the controls it shows the active tab's address; clicking it — or `Cmd/Ctrl+L` — turns the chip into a live search/URL field *in the sidebar*, prefilled + selected, with the same unified autocomplete dropping down over the tab list. Commit loads in the active tab; Escape or clicking away restores the display and returns focus to the page. Only the on-screen sidebar hosts the live field (a collapsed sidebar reveals first). With no active tab the chip is a placeholder that opens the command bar for a new tab. This is the one persistent entry surface the guide allows — it's the sidebar-width chip, never the full-width toolbar §3/§13 forbid.

---

## 7. Motion

Snappy, native, purposeful. Motion communicates state change; it never delays the user.

- **Use native spring/easing systems** — SwiftUI springs / Core Animation on macOS, Composition on Windows. Don't hand-roll timing curves that fight the platform.
- **Animate these moments** (identically across platforms, §2): tab create/close/reorder/select, sidebar collapse/expand, space switch, command-bar expand/dismiss, hover/focus transitions.
- **Never block interaction on animation.** The user can act mid-animation; input is never queued behind a transition. Keep durations short (roughly system-standard); prefer interruptible springs.
- **Honor reduce-motion.** When the OS requests reduced motion, cross-fade or cut instead of moving/scaling. This is a correctness requirement, not a nicety (§11).

---

## 8. Iconography & typography

- **Native icon sets only:** SF Symbols (macOS), Segoe Fluent Icons (Windows), the platform set on Linux. Pick the glyph that carries the same *meaning* on each platform; don't ship a custom icon font to force pixel-identical icons.
- **System fonts, system sizing.** Use the platform UI font and default type scale; respect Dynamic Type / OS text scaling (§11). No bundled brand font for chrome.
- **Weight and density** favor a power-user read: legible but compact. Tabular figures where numbers align (e.g. any counts).

---

## 9. Keyboard & power-user

Everything is reachable from the keyboard; the command bar is the hub. Shortcuts follow Safari/Chrome/Arc so muscle memory transfers. macOS uses Cmd; Windows/Linux use Ctrl for the same actions.

| Action | Shortcut (macOS) |
|---|---|
| Open command bar (new tab) | `Cmd+T` |
| Open command bar (edit current URL) | `Cmd+L` |
| Close tab | `Cmd+W` |
| Reopen last closed tab | `Cmd+Shift+T` |
| Next / previous tab | `Cmd+Shift+]` / `Cmd+Shift+[` |
| Select tab 1–8 / last tab | `Cmd+1`…`Cmd+8` / `Cmd+9` |
| Move tab up / down in list | `Cmd+Opt+Shift+↑` / `↓` |
| Switch space (next / prev) | `Cmd+Opt+→` / `Cmd+Opt+←` |
| Toggle sidebar | `Cmd+S` |
| Reload / hard reload | `Cmd+R` / `Cmd+Shift+R` |
| Back / forward | `Cmd+[` / `Cmd+]` |
| Show history | `Cmd+Y` |
| Find in page | `Cmd+F` |
| Snip an element (Tweaks) | `Cmd+Shift+Y` |
| DevTools (toggle) | `Cmd+Opt+I` / `F12` |
| Inspect element | right-click → *Inspect Element* |
| Fullscreen | `Cmd+Ctrl+F` |

- **Keep this table the source of truth.** Add a shortcut here before wiring it, and mirror it on every platform (Cmd↔Ctrl). Don't invent per-platform-only shortcuts for shared actions.
- **No dead ends.** If an action exists in a menu or on hover, it has a shortcut. Power users should be able to drive tabs, spaces, and navigation without the mouse.

---

## 10. States & feedback

Every surface handles every state. No placeholder, no "for now."

- **Loading:** per-tab spinner in the sidebar row and a native progress affordance; never a frozen or blank chrome.
- **Error:** custom, native-styled error pages for navigation failures (ALLOY runtime path, spec §7) — clear, actionable, not a raw Chromium interstitial where we can do better.
- **Empty:** a space with no tabs, no history yet, empty command-bar results — each has a designed empty state, not blankness.
- **Hover / focus / active / disabled:** every interactive control shows all four correctly, using native control states. Focus rings follow the platform; keyboard focus is always visible.
- **Feedback is immediate.** Actions acknowledge within a frame or two — optimistic where safe, with the core as the source of truth.

---

## 11. Accessibility

Going native buys platform accessibility largely for free (spec §14) — don't squander it.

- **Use native controls** so VoiceOver / Narrator / screen readers, and UI Automation work without a custom a11y tree. Custom-drawn chrome would forfeit this; avoid it.
- **Honor system settings:** reduce-motion (§7), increased contrast, text scaling / Dynamic Type (§8), and reduce-transparency (fall back off glass to a solid material when the OS asks).
- **Full keyboard navigation** (§9) and visible focus everywhere.
- **Label everything** — icon-only controls carry accessible labels; tab rows announce title and load state.

---

## 12. Terminology

One vocabulary across UI copy, code, and all platforms. Consistency in naming is part of the consistency contract (§2).

| Use | Not |
|---|---|
| **tab** | page (for a tab) |
| **space** | workspace, profile, group |
| **sidebar** | tab bar, rail, panel |
| **command bar** | omnibox, address bar, palette, launcher |
| **web view / content** | viewport, page area |
| **Tweak** (a per-site customization) | boost, mod, site rule |
| **Snip** (the element remover) | zap, element picker/zapper (in UI copy) |

- **"Mod"** is reserved for browser-*chrome* theming (a later, separate feature); page-content customization is always a **Tweak**. **"Script"** is the JS tier of Tweaks; *userscript* is its technical name, not the user-facing umbrella.

- Match this in identifiers and FFI names too (`switch_space`, `command_bar_query`, …) so code and UI speak the same language.

---

## 13. Anti-patterns

What we deliberately don't do. Reviewers should reject these on sight.

- **No Arc-style thick inset frame** — no rounded content card with big margins (§3).
- **No persistent full-width address bar** — new-tab entry is the transient command bar; current-tab editing is the inline sidebar chip (§6, ADR-0014). Neither is a full-width toolbar: the chip lives in the sidebar's width and costs no content area.
- **No custom-rendered chrome** — no Vello/GPU-drawn buttons, tabs, or windows (spec §1). Native controls only; this is also what buys accessibility (§11).
- **No per-platform behavioral divergence** for shared actions — same model, same shortcuts, same flows everywhere (§2). Native skin, not native behavior differences.
- **No glass on content rows / stacked glass backdrops** — glass is the top functional layer only (§3).
- **No web-tech chrome** — the chrome is native, not an HTML/CSS surface rendered by CEF. This includes the **Snip picker**: the highlight and its affordances are a native overlay above the web view, never HTML/CSS injected into the page (§14).
- **No silent auto-run of imported Tweaks** — importing never enables; a Script (JS) Tweak shows its code and requires explicit consent before it runs (§14).
- **No hidden or obfuscated Tweak code** — a Tweak's CSS/JS is always inspectable before and after enabling.
- **No placeholder states shipped** — see §10; flag instead of shipping half.

---

## 14. Tweaks — page customization (Snip / Restyle / Script)

Per-site page customization (spec §20). Three tiers over one model: **Snip** (click to remove an element, no code), **Restyle** (custom CSS), **Script** (custom JS, power users). The core owns the rule store and scoping; the UI is the picker, the editor, and the manager.

**Snip mode (the element remover).**

- **Enter** from the command bar ("Snip an element") or `Cmd+Shift+Y` (§9). The pointer enters a picking state and the tab is clearly in snip mode — a native mode indicator, not a web-styled banner.
- **Hover highlights** the element under the pointer with a crisp, accent-colored native overlay rectangle plus a small label (tag + dimensions). Highlight *geometry* is read from the page; the highlight and every affordance are **native chrome drawn above the web view**, never HTML/CSS injected into the page (§13).
- **Widen / narrow** the selection up and down the DOM tree with `↑` / `↓` (uBO-style), so the user grabs the right container, not just the leaf node.
- **Commit** with click / Enter → the element hides immediately and the rule is saved to this site's Tweak. **Escape** cancels with nothing saved. A committed Snip is undoable.
- The generated selector is robust (prefers stable ids/classes, falls back to structural position) and shown to the user, editable later in the manager.

**Scope.** A new Tweak defaults to the current site, shown plainly ("this site"). Power users open an advanced field to edit the match pattern (spec §20). Don't force pattern syntax on the common case; don't hide it from the power case.

**The editor (Restyle / Script).** A native code surface — monospace, a match-pattern field, and the CSS or JS body — reached from the manager or "Edit as CSS / JS" on an existing Tweak. Live-applies on save. The common Snip path never opens an editor; the editor is opt-in for people who want it.

**Managing Tweaks.**

- **Per-site:** a panel (from the command bar or the site menu) listing this site's Tweaks with per-Tweak toggles and an "Add" affordance (Snip / Restyle / Script).
- **Global:** a Tweaks manager in settings — every Tweak across all sites, grouped by site: enable / disable / edit / delete / import / export.
- **Active indicator:** when any Tweak is applied to the current page, show a quiet native indicator — a stronger one for a Script Tweak. The user always knows a page is being modified.

**Safety UX (Script tier).** Custom JS is arbitrary code in an authenticated session (spec §20 · rm §8.4) — the UI carries the guardrails:

- **Enabling or importing a Script Tweak shows the code first** and requires an explicit, plainly-worded confirmation; a changed script re-prompts. Never enable an imported Tweak silently.
- **Snip / Restyle are lighter-gated** (no DOM or script access) but still shown before enable.
- A **global kill switch** disables all Tweaks at once; per-site and per-Tweak toggles are always one action away.

**States.** The empty manager has a designed empty state that explains Snip, not blankness. A Tweak whose selector no longer matches (stale after a site redesign) is surfaced as inactive with an offer to re-Snip or delete — never a silent no-op. Reduce-motion and reduce-transparency apply to the picker overlay like any chrome (§7, §11).

**Keyboard.** Snip mode is `Cmd+Shift+Y`; within it `↑` / `↓` widen / narrow, Enter commits, Escape cancels. Opening the manager, adding a Restyle / Script, and toggling are all reachable from the command bar (§6). No mouse-only paths.

---

## 15. Internal pages (History, Settings, Bookmarks, Downloads)

App-UI surfaces are **native**, not web pages (ADR-0010). Each renders in the content area as a *surface* shown over the web content and dismissed back to the active tab (like the empty state, §10) — a page-like placement (`Cmd+Y` opens History), **not** a `browser://` URL and **not** a tab. Web/internal HTML is reserved for what must render inside the web viewport (error pages, future reader-mode/PDF).

- **Shared shell.** A **glass functional top bar** (title + search + primary actions) over a **native content region**. The bar is the macOS 26 toolbar idiom: a large title plus Liquid Glass **capsule controls** (search field, action buttons) floating directly over the content via `safeAreaBar` — content scrolls under it with the system scroll edge effect. No stacked bar material behind the controls (§3, §13); glass lives on the controls themselves, and content rows stay native. The shell is common across every internal page so they read as one family — History sets it, the rest follow.
- **Content.** Native `List` / controls (§8, §11): favicon + primary/secondary text + a trailing, tabular metadata column (§8); native section headers where the data groups naturally (History by day).
- **States (§10).** A designed empty state, a distinct no-results state for a search, and hover-revealed row actions (e.g. remove).
- **Dismissal.** Escape, `Cmd+W`, or opening/navigating to a tab closes the surface — an open surface intercepts `Cmd+W` so it never closes the tab or window beneath it. `Cmd+Y` toggles History.
