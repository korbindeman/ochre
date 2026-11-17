# ADR-0002: Tweaks - one page-customization engine, three tiers, on the adblock injection path

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

We want Arc Boosts-style per-site page customization - primarily *removing* page elements
(a visual "zap"-style picker), plus custom CSS, and custom JS for power users. It is framed as a
pillar of the browser's **"extensible by scripting and automation"** goal, not a one-off feature.

Three things in the existing design already overlap this space and force a decision about how they
relate:

- **Adblock cosmetic filtering** (spec §11) already injects element-hiding CSS + scriptlets per page
  through the CEF request/load path.
- **The userscript engine** is already committed as the "native-absorb" answer to extensions
  (roadmap §3.1, Phase 2–3) via CDP `Page.addScriptToEvaluateOnNewDocument`.
- The **roadmap taxonomy** listed "Boosts/per-site CSS" and a "userscript engine" as *separate*
  line items, with no stated relationship - the thing this ADR resolves.

The real question is not "should we build it" (yes) but **how many mechanisms and what to call them** -
and how to handle the security of running user- or agent-supplied JS in an authenticated session.

## Decision

Build **one** subsystem - **Tweaks** - as three UX tiers over a **single** primitive:
a URL-scoped store of content injected at document-start.

- **Tiers:** **Snip** (visual element remover → an element-hiding CSS rule), **Restyle** (custom CSS),
  **Script** (custom JS in an isolated world). One match-pattern scoping model serves all three.
- **Mechanism:** Tweaks **reuse the adblock cosmetic-injection path** (spec §11), not a parallel one.
  A Snip is mechanically a user-authored cosmetic rule; filter-list rules and user Tweaks coexist in
  one injector, differing only in provenance. New core module `tweaks` owns the store, scoping,
  editor state, and import/export; it emits through the same `engine` hook `adblock` feeds.
- **Naming (canonical, in the terminology contract):** the feature/unit is a **Tweak**; the element
  remover is **Snip**. *Not* "Boost" (Arc's term), *not* "Mod" - **"Mod" is reserved for browser-chrome
  theming**, a separate later feature. "Script" is the JS tier; "userscript" is its technical name only.
- **Safety posture for Script (JS):** no silent auto-run of imported Tweaks; code is always shown and
  diffable before enable (and re-prompted on change); JS requires explicit worded consent; run in an
  **isolated world**; global kill switch + per-site/per-Tweak toggles + an on-page active indicator.
  Isolation is treated as *not* a security sandbox against the user's own session - the consent gate is
  the control.
- **Sync-ready from the start** (a Tweak is a portable document), shipping behind sync later (roadmap §3.4).

Documented in spec §20 (structure), the UI style guide's Tweaks section (UX), and roadmap §3.1 / §4 /
Phase 2–3 (strategy).

## Alternatives

- **A separate "content-scripts" subsystem parallel to adblock** - rejected: it would duplicate the
  document-start injection, per-page scoping, and CSS/scriptlet plumbing adblock already has, for no
  benefit. Provenance (user vs. filter-list) is a data attribute, not a reason for a second engine.
- **Three independent features** (an element zapper, a "custom CSS" tool, a userscript manager) -
  rejected: three scoping models, three UIs, three stores for what is one primitive. The unified
  "one primitive, three tiers" framing is the same leverage the roadmap prizes elsewhere (rm §5.3).
- **Adopt Arc's "Boosts"/"Zap" names** - rejected (product call): borrows a competitor's brand term.
  We use our own vocabulary; "Boosts" stays only as an external reference point.
- **Fold everything under "Mods"** (one word for content + chrome theming) - rejected: it blurs
  "customize this site" with "theme the browser," which are different features with different scopes.
- **CSS/element-hiding only, no JS** - rejected: it abandons the power/developer audience and the
  explicit scripting-extensibility goal. JS is included, but gated (see Consequences).

## Consequences

- **Cheap to build the tiers.** Once the shared injector + scoping + store exist, Snip, Restyle, and
  Script are mostly UI. The element picker (robust-selector generation, DOM-tree widen/narrow) and the
  code editor are the main net-new UI surfaces.
- **Concrete extensibility hook.** Because a Tweak is a declarative, inspectable document, the agent/MCP
  layer (spec §19) reads, proposes, and applies Tweaks - human userscripts and agent-authored Tweaks
  are the *same object*, which is what makes the extensibility goal real rather than aspirational.
- **A permanent security surface we own.** Running user/agent JS in authenticated sessions is the
  userscript supply-chain risk (sibling to AI prompt-injection, rm §8.4). The consent gate, code
  visibility, and kill switch are load-bearing, not polish - a regression here is a security bug.
- **The Snip picker is one of the few places chrome touches content.** Highlight geometry is read from
  the page, but the overlay and affordances must stay native (no web-tech chrome) - a specific
  constraint recorded in the style guide.
- **Sync must carry Tweaks** when it lands; the model is designed portable now to avoid a retrofit.
- **A stale-selector failure mode** (a Snip breaks after a site redesign) must be surfaced, not silently
  no-op - an explicit UX state.
