# ADR-0040: Custom CEF build with proprietary codecs (revises the rm §3.2 OS-decoder posture)

- **Status:** Accepted
- **Date:** 2026-07-09

## Context

The stock Spotify prebuilt CEF (what `cef-dll-sys` downloads) ships with proprietary codecs
**disabled** - no H.264/AVC, no AAC. In daily dogfooding this is now a visible product failure: a
lot of sites simply don't play video ("video doesn't play" fails the complete-browser bar,
rm §4.2). Roadmap §3.2 recommended an OS-decoder posture (VideoToolbox/Media Foundation, FFmpeg
proprietary paths off) to avoid patent exposure, and left the build-vs-prebuilt question open for
Phase 2.

Research (2026-07-09) closed both halves of that question:

- **No usable prebuilt exists.** The only public prebuilt CEF with codecs on macOS arm64 is OBS
  Studio's ([obsproject/cef](https://github.com/obsproject/cef), verified empirically - their
  binary contains the `proprietary_codecs`-gated code paths ours lacks), but it's pinned to
  **CEF 127** with OBS-specific OSR patches - ABI-incompatible with our 148-line `cef` crate
  bindings, updated on OBS's schedule. Spotify's builds exclude codecs by standing policy
  ([cef#3559](https://github.com/chromiumembedded/cef/issues/3559) - the OS-decode compromise -
  has not landed and found AAC can't be routed through OS APIs cleanly); JetBrains JCEF
  (JBR-2368) and CefSharp refuse for licensing reasons; jcefbuild/Chromely repackage Spotify's
  binaries; JxBrowser/castLabs ship codecs but aren't CEF.
- **A custom build is cheap.** CEF's AutomatedBuildSetup with
  `GN_DEFINES="proprietary_codecs=true ffmpeg_branding=Chrome"` is a well-trodden path producing
  a standard binary distribution. On an ephemeral cloud Mac (Scaleway Mac mini M4, €0.29–0.49/h,
  Apple-mandated 24 h minimum) a full checkout + build + package fits in half a day: **~€7–12
  per build**, ~€12 per future CEF bump with a cached checkout tarball. (AWS EC2 Mac is 4–6× the
  price; GitHub Actions macOS runners are disqualified on disk - 14 GB SSD vs ~150 GB needed; no
  distributed-build shortcut exists for outsiders - Google's remote-exec is invite-only, sccache
  is empty on a first build.)
- **Widevine doesn't change the calculus but depends on this.** Modern CEF fetches the Widevine
  CDM at runtime with no custom build, but decrypted streams from real services are H.264/AAC -
  DRM playback additionally requires these codecs built in. The DRM *license* posture (rm §3.2:
  no premium-DRM promise at launch) is unchanged by this ADR.

## Decision

Build our own CEF binary distribution from source with `proprietary_codecs=true
ffmpeg_branding=Chrome`, on the current pin line (148.4.0, ADR-0028), on an ephemeral cloud Mac
(Scaleway Mac mini M4 class), and point `cef-dll-sys` at it (`CEF_PATH`) instead of the Spotify
prebuilt. The build is scripted and repeatable; every future CEF bump (already gated by
ADR-0028's redirect-selftest) rebuilds through the same pipeline.

We accept the patent posture deliberately: AVC/H.264 sits under Via LA's 100k-units/year
royalty-free tier - a non-issue for a long time; AAC has no free tier, and we knowingly carry
that exposure at pre-launch scale (as OBS and many small projects do). Revisiting codec licensing
becomes a scale-gated item, not a blocker.

## Alternatives

- **Any prebuilt with codecs** - rejected: none is viable. OBS's is the only one on macOS arm64
  and it's CEF 127 + OBS patches (ABI-incompatible, 21 majors behind, someone else's cadence);
  everything else is codec-less, dead, Linux-only, or not CEF at all (see Context).
- **OS-provided decoders, FFmpeg proprietary paths off** (the prior rm §3.2 recommendation) -
  rejected: it's a real engineering project with a known-unsolved core problem (AAC audio cannot
  be cleanly routed through OS decode APIs - the finding that stalled cef#3559), versus ~€10 and
  a scripted build day. Cleaner on patents, wrong trade at this scale.
- **Stay codec-less until Phase 2 scope-lock** - rejected: the breakage is a daily dogfooding
  cost *now*, and the decision was always going to land on a custom build (rm §4.2 already
  predicted it for API keys + Web Push); deferring buys nothing.
- **Bundle HEVC too** - rejected (unchanged from rm §3.2): the multi-pool, rising-royalty HEVC
  landscape has no free tier and mainstream web video doesn't require it. HEVC stays OS-side or
  absent.
- **AWS EC2 Mac / MacStadium / GitHub Actions for the build machine** - rejected on cost/shape:
  EC2 Mac is 4–6× Scaleway for the same 24 h minimum; MacStadium is monthly-minimum; GitHub
  Actions macOS runners physically can't hold the checkout (14 GB SSD).

## Consequences

- Mainstream MP4/H.264/AAC web video works; the single biggest site-compat gap closes. AV1/VP9
  continue to work as before.
- **We own a build pipeline**: a scripted, documented cloud build per CEF bump (~€12, ~half a
  day wall-clock) replaces "download the prebuilt". The artifact must be hosted somewhere the
  build can fetch it, and `CEF_PATH` wiring must not break contributor onboarding (document the
  fallback to the stock prebuilt for hacking on non-media features).
- **Patent exposure is accepted, not eliminated**: AVC royalty-free under 100k units/yr; AAC has
  no free tier. File a scale-gated revisit (licensing or posture change) before distribution
  numbers matter; never bundle HEVC without a new ADR.
- The same pipeline is the natural carrier for the rest of the rm §4.2 source-build cluster -
  Google API keys (Safe Browsing, geolocation) and Web Push/FCM - as follow-on work.
- DRM: Widevine CDM runtime-fetch now has codecs to decode into, but the license/VMP wall
  (rm §3.2) still stands - no premium-DRM promise at launch, unchanged.
- Standing regressions (`just redirect-selftest`, `just focus-selftest`, `just close-selftest`,
  `just drag-selftest`) must run against the custom binary before it replaces the prebuilt -
  same bar as a CEF version bump.
