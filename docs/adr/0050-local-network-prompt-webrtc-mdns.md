# ADR-0050: Local-network prompt root cause (WebRTC mDNS); Cast discovery disabled as intent

- **Status:** Accepted
- **Date:** 2026-07-16

## Context

Users saw macOS 15+'s "find devices on your local network" permission prompt appear
seemingly at random, with no explanation of why the browser wanted it (user requirement
2026-07-16: no scary OS permission asks the user can't connect to a feature).

The obvious suspect was Chromium's Media Router (Google Cast), which CEF inherits
enabled by default and which in stock Chrome starts mDNS + DIAL multicast device
discovery when a page uses the Presentation API. **Instrumented testing disproved
this**: under the ALLOY runtime on CEF 148, a page calling
`PresentationRequest.getAvailability()` resolves `false` immediately and the process
opens **no** discovery sockets (verified via `lsof` on UDP 5353/1900 across the whole
process tree, with the MediaRouter feature still enabled). Cast discovery is
effectively inert in our hosting mode today.

The **verified trigger is WebRTC**: the moment any page opens an `RTCPeerConnection`
(video-call sites, but also fingerprinting and telemetry scripts on ordinary pages),
Chromium's default-on mDNS local-IP obfuscation (`WebRtcHideLocalIpsWithMdns`)
registers a `<uuid>.local` hostname for the ICE host candidate instead of exposing the
LAN IP. That registration binds a set of UDP 5353 sockets in the network service and
multicasts - which is exactly what macOS 15+ flags as local network access. Reproduced
end-to-end: a test page's ICE candidate came back as `<uuid>.local` and the network
service opened ~12 UDP 5353 sockets. This also explains the "sometimes" pattern: it
fires on the first WebRTC-using page, not at launch.

## Decision

Two-part posture:

1. **Disable the Media Router** via `--disable-features=MediaRouter,DialMediaRouteProvider`
   in both command-line hooks (`app.rs` standalone, `embed.rs` embedded). This changes no
   observable behavior today (discovery is already inert under ALLOY); it encodes the
   product intent - no OS-permission-consuming capability before the feature it powers
   ships - and guards against a future CEF bump silently activating discovery. It comes
   out only as part of a full Cast slice: picker UI + pre-permission explainer + parity
   row (backlog P2-44).

2. **Keep WebRTC's mDNS local-IP obfuscation enabled** and accept its one-time OS prompt.
   The alternative (disabling `WebRtcHideLocalIpsWithMdns`) would silently expose the
   user's LAN IP to every site that opens a peer connection - a real privacy regression
   spent to avoid a single, OS-mediated, deniable prompt. Denying the prompt degrades
   nothing important: mDNS candidates simply fail to resolve and WebRTC falls back to
   STUN/relay paths; only same-LAN peer-to-peer shortcuts are lost. This matches what
   Chrome/Edge/Brave ship on macOS 15+.

## Alternatives

- **Disable `WebRtcHideLocalIpsWithMdns` to kill the prompt entirely** - rejected: it
  trades a one-time, user-deniable OS prompt for a permanent silent local-IP leak to any
  site running a WebRTC fingerprinting snippet. Wrong direction for a browser whose
  posture is native adblock + privacy.
- **A pre-permission explainer before the OS prompt** - rejected as infeasible: the
  prompt fires on first socket use deep inside Chromium's network service; macOS offers
  no reliable pre-prompt hook, and the browser cannot tell "about to prompt" from
  "already granted/denied".
- **Restrict WebRTC candidates (`webrtc.ip_handling_policy`)** - rejected: the
  default-route-only policy still produces a host candidate that gets mDNS-obfuscated,
  so the prompt fires anyway; stricter policies (`disable_non_proxied_udp`) break real
  calls.
- **Remove the MediaRouter switch since discovery is inert anyway** - rejected: one line
  with a clear comment is cheap insurance against a CEF bump flipping ALLOY's media
  router support on; without it the prompt would regress with no code change on our side
  and no test to catch it.
- **Do nothing at all** - rejected; the investigation itself is the value: the causal
  story is now recorded, and the intent switch prevents a second, genuinely
  Ochre-caused trigger from appearing later.

## Consequences

- The local-network prompt can still appear - once, on the first page that uses WebRTC.
  This is a deliberate privacy-over-polish call; the prompt is OS-mediated and safe to
  deny (calls still work via STUN/relay).
- Casting to Chromecast/DIAL devices is deliberately unsupported until P2-44; sites'
  cast buttons report no available devices, which is honest - there is no picker UI.
- A future CEF bump that activates ALLOY media-router discovery is pre-neutralized by
  the switch rather than silently re-introducing multicast.
- If a "never prompt" stance is ever wanted, it must be a user-visible setting that
  states the local-IP-exposure tradeoff, not a silent default flip.
