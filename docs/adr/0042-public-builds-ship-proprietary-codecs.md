# ADR-0042: Public browser builds ship proprietary codecs — a conscious pre-launch risk, codec-less as the one-flag retreat

- **Status:** Accepted
- **Date:** 2026-07-09
- **Supplements:** ADR-0040 (which decided the *build* posture; this decides the *distribution* posture)

## Context

ADR-0040 decided we compile a custom CEF with `proprietary_codecs=true ffmpeg_branding=Chrome`
and framed the H.264/AAC patent exposure as "private/quiet at pre-launch scale." That framing
assumed the codec-enabled binary stayed close to us. It doesn't survive contact with the actual
distribution plan: **`ochre` is a public repo, and we intend to publish public browser releases
built by GitHub Actions.**

The patent-license obligation for H.264/AAC decoding attaches to **whoever ships the decoding
software to end users** — here, the release publisher — not to where a build artifact is stored.
So a public browser release that embeds the codec-enabled framework *is* a public redistribution
of H.264/AAC decoders under the maintainer's name, regardless of the CEF tarball living in a
private artifact repo. Keeping the raw tarball private reduces exposure **only** if the public
builds are codec-less. This is a distribution decision ADR-0040 did not actually make.

The forces:
- **AVC/H.264** is royalty-free under Via LA's **100,000-units/year** tier — comfortable at our
  scale for a long time.
- **AAC has no royalty-free tier** — technically owed from unit one. This is the naked exposure,
  and public release makes it attributable to a named individual on GitHub.
- The product thesis (roadmap) is a *usable* browser. Codec-less public builds mean video doesn't
  play — the exact problem ADR-0040 set out to solve stays unsolved for public users.
- Many small OSS projects (OBS being the canonical example) ship proprietary codecs publicly and
  accept the risk; enforcement against hobby-scale free software is near-zero in practice.

## Decision

**Public browser builds ship with proprietary codecs**, as a conscious calculated risk taken
while the project is pre-launch and small. Concretely:

- The **GitHub Actions maintainer release workflow** (running on `ochre` with a read-only
  fine-grained PAT for the private artifact repo `korbindeman/ochre_cef` in repo secrets) fetches
  the codec-enabled CEF framework, builds `Browser.app`, and publishes the codec-full public
  release. (A PAT is required because the public repo's built-in `GITHUB_TOKEN` cannot read a
  separate private repo.)
- **Fork-PR CI cannot reach the private artifact** (GitHub withholds secrets from fork PRs), so
  contributor builds fall back to the stock codec-less Spotify prebuilt automatically. This gives
  a clean **maintainer-codecs / contributor-codecless** split for free.
- **Codec-less is the documented one-flag retreat.** The fetch/build path keeps the stock-prebuilt
  fallback as a first-class configuration, so we can revert public builds to clean-hands the moment
  scale, attention, or the AAC exposure becomes uncomfortable — without an architecture change.
- The **raw CEF-with-codecs tarball stays in the private `ochre_cef` repo** (release assets, one
  per CEF version) — not for patent protection (the app makes the codecs public anyway) but to
  avoid becoming a convenient standalone CEF-with-codecs *upstream* others pull from, i.e. avoiding
  an additional redistribution beyond our own app.

This is **not** a licensing clearance and **not** legal advice. It is an explicit, logged
risk-acceptance at hobby scale, to be revisited before distribution numbers or visibility grow.

## Alternatives

- **Codec-less public builds** (the Chromium / ungoogled-chromium path) — rejected *for now*:
  clean on patents, but ships the exact broken-video experience we're trying to fix; public users
  would have to side-load codecs. Kept available as the retreat, not the default.
- **Pretend private artifact hosting neutralized the exposure** — rejected as false: the codecs
  reach the public through the app regardless of where the artifact is stored. Recording the real
  posture is the point of this ADR.
- **Pay AAC per-unit royalties for a free product** — rejected as absurd at this scale; the point
  of the retreat flag is to have a clean exit rather than a royalty stream.
- **Bundle HEVC** — still rejected (ADR-0040): worse, multi-pool, no free tier, not needed.

## Consequences

- Public users get a browser where mainstream H.264/AAC video plays — the thesis holds.
- The maintainer (release publisher) carries the AAC exposure publicly and by name. Accepted
  consciously; **file a scale-gated revisit** — reassess (retreat to codec-less, or pursue a
  license) before download volume or public visibility grows meaningfully.
- The GHA release workflow needs a read-only PAT for `ochre_cef` as a repo secret; the fetch script
  must degrade to the stock prebuilt when the token is absent (contributor path) — this is a hard
  requirement, not a nicety, and is what makes the maintainer/contributor split work.
- The retreat must stay real: the codec-less build path is tested/kept working, not left to rot,
  so "flip back to clean-hands" is genuinely one flag.
- Nothing here changes the DRM posture (ADR-0040 / rm §3.2): no premium-DRM promise at launch.
