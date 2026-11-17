# ADR-0026: Cookie-consent - reject-first, hide as an honest fallback

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

EU cookie-consent banners (GDPR/ePrivacy CMPs - OneTrust, Cookiebot, Didomi,
Quantcast, Sourcepoint, TrustArc, and the IAB **TCF** framework) are a top
browsing annoyance and an explicit native-absorb target (`rm §3.1`). This is the
*consent-popup* problem, not HTTP `Set-Cookie` handling. Two mechanisms exist:

- **Hide** - element-hide the banner like an ad. Nearly free: it rides the
  cosmetic-filtering engine (P2-21) with cookie-annoyance lists. **But hiding does
  not answer the banner** - many sites default non-essential cookies to *on* until
  you reject, so the user is still tracked while the UI merely *looks* clean.
- **Reject** - drive the CMP to "reject all" (or set the consent state to
  rejected). The privacy-real behavior, and the harder engineering.

The fork is a **values** decision, not just an engineering one. Hiding-without-
rejecting presents a privacy-clean face while leaving the user tracked - a mild
dark pattern *against our own users*, and precisely the criticism of the original
"I Don't Care About Cookies" (which sometimes *accepts* to clear the banner). Our
whole wedge is "local-first, transparent, trustworthy" (`rm §1.5 / §3.1`), so the
posture here has to match the brand, not just clear pixels.

## Decision

- **Reject-first.** Where a CMP is recognized, actually opt out (reject all
  non-essential). **Hide is a fallback for the long tail only**, and the product
  is honest that *hidden ≠ rejected*. We never ship hide-alone dressed as privacy.
- **Layer them, don't choose per-site:** hide the banner visually at
  document-start (instant, no flash) **and** reject underneath in the background.
  Clean UI *and* a real opt-out, and it kills the flash pure-reject suffers.
- **Mechanism:** embed **Consent-O-Matic**'s (MIT) per-CMP ruleset + operator,
  injected via the P2-14 substrate. For IAB-**TCF** CMPs, prefer setting the
  consent string to all-rejected via `__tcfapi` over DOM-clicking (robust,
  standardized). Long-tail hide comes from cookie-annoyance filter lists on the
  P2-21 engine.
- **Hot-update the ruleset** like the adblock lists (ADR-0020): bundle a snapshot,
  background-refresh, hot-swap.
- **Licensing:** Consent-O-Matic is MIT ✓. Use the uBO-maintained *"I Still Don't
  Care About Cookies"*, **not** the Avast-owned IDCAC original. EasyList Cookie is
  runtime data (GPLv3 / CC-BY-SA) - verify distribution terms before vendoring.

## Alternatives

- **Hide-only (IDCAC-style).** Rejected as the *primary* posture: cheap and broad,
  but leaves the user tracked behind a privacy-clean facade - it contradicts the
  trust thesis and is the exact anti-pattern we position against. Kept solely as
  the honest long-tail fallback, labeled as such.
- **Reject-only.** Rejected as the sole mechanism: no ruleset covers the whole
  long tail, and pure reject flashes the dialog before dismissing it. Hiding
  layered underneath fixes both.
- **Pure DOM-clicking for TCF CMPs.** Rejected where the TCF API is available:
  fragile DOM automation (breaks when the CMP restyles) vs. the standardized
  `__tcfapi` consent-string path. Fall back to DOM automation only off-TCF.

## Consequences

- **Standing maintenance cost.** CMPs are adversarial and the ruleset rots - budget
  Tier-B (reject) like the CEF-security and adblock-list treadmills, not one-and-done.
- **Forces an all-frames substrate extension.** CMP dialogs render in iframes (TCF
  uses a `__tcfapiLocator` frame), so the P2-14 substrate - today main-frame only -
  must inject into subframes for reject to work. A prerequisite for Tier B, filed
  as discovered work.
- **Hide and reject interact mechanically.** You can't `display:none` a banner and
  then click its reject button - the hide must be interaction-preserving (or lifted
  before the reject interacts). The two tiers are co-designed, not independent.
- **Sequenced after P2-21.** Tier A (hide) rides the cosmetic engine's lists; Tier B
  (reject) rides the substrate. So the order is P2-21 (cosmetic ad-block) → P2-22
  (cookie-consent), and P2-22's first real cut is Tier B for the top CMPs + Tier A
  underneath.
