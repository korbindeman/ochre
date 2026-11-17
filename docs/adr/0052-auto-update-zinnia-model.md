# ADR-0052: Auto-update - Zinnia model (GitHub + minisign + sidebar card)

- **Status:** Accepted
- **Date:** 2026-07-19

## Context

Security patches on a CEF browser are worthless without a path that gets them onto
users' machines. The roadmap already called for Sparkle 2.x (rm §6.2) as the macOS
end-state, but that depends on Developer ID signing + notarization (P0-2, still
blocked on Apple Developer enrollment). Meanwhile alpha builds already ship as
GitHub pre-releases, and a prior product (Zinnia) already solved dogfood updates
with a simpler, signed-archive design that works without Apple code signing.

User ask: when a new version is detected, show a sidebar dialog (Arc / Claude style)
with **Update & Restart**.

## Decision

Copy **Zinnia's update model**, adapted to this stack:

1. **Manifest:** Tauri-compatible `latest.json` on GitHub Releases (`version`,
   `notes`, `pub_date`, `platforms.<key>.{url,signature}`).
2. **Trust:** minisign over the archive; public key compiled into the core; private
   key only in CI secrets (`UPDATE_SIGNING_PRIVATE_KEY`).
3. **Package:** `Browser.app.tar.gz` (full app bundle), not Sparkle deltas yet.
4. **Core-owned pipeline** (`crates/core/src/update.rs`): check → download → verify →
   extract → spawn a detached replace-and-relaunch helper → host quits.
5. **Host UI:** persistent sidebar card + **Browser → Check for Updates…**;
   "Updated to vX" + Release Notes after a successful relaunch.
6. **Prerelease alphas:** `/releases/latest` ignores prereleases, so the client
   falls back to the GitHub Releases API and picks the first non-draft release that
   has a `latest.json` asset.

Sparkle (with EdDSA + binary deltas) remains the **long-term** macOS path once
signing lands - this ADR does not replace that roadmap item; it ships a working
dogfood channel now without inventing a one-off trust model.

## Alternatives

- **Sparkle 2.x only** - rejected for now because apply/restart is unreliable on
  unsigned ad-hoc alphas, and P0-2 is still blocked. Revisit when Developer ID
  + notarization land; deltas still matter for a ~200 MB CEF bundle.
- **Unsigned GitHub zip replace** - rejected: no payload authenticity beyond HTTPS.
  Minisign matches Zinnia/Tauri and is one secret to store.
- **Tauri plugin as a dependency** - rejected: we are not a Tauri app; reusing the
  **manifest + minisign format** is enough without pulling the plugin runtime.
- **Core opens the release page only** - rejected: user asked for Update & Restart
  end-to-end, not a half-version.

## Consequences

- Release workflow must produce `Browser.app.tar.gz` + signed `latest.json` and pin
  `MARKETING_VERSION` to the tag so version compares are honest.
- Repo secret `UPDATE_SIGNING_PRIVATE_KEY` (and optional password) is required for
  installable updates; missing secret publishes an unsigned manifest and verify fails
  closed (card may still appear if an older signed release exists).
- Full-archive downloads (no deltas) are acceptable for alpha; bandwidth tax grows
  with CEF size - Sparkle deltas stay the Phase-2 goal.
- Windows later reuses the same manifest/signature scheme; install helper is
  platform-specific (only macOS is implemented here).
- Dev (`.dev` bundle id) skips auto-check noise; menu check still works.
