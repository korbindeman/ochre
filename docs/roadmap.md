# Roadmap

The strategy layer - MVP to top-tier - above `architecture.md` (the *how*) and `mvp1.md` (the first slice).
This answers: **what does it take to make this a serious, releasable open-source browser - for power users,
developers, and people who love beautiful, clean browsers - and in what order do we build it?**

Cross-refs: bare `§N` = this doc; `spec §N` = architecture.md; `mvp` = mvp1.md. Where this doc revisits a
decision locked in the spec, it says so explicitly and presents options + a recommendation - the call is yours.

---

## 1. Thesis & positioning

### 1.1 The uncontested lane

The "**beautiful, clean, open-source browser**" lane is currently held by exactly one credible product -
**Zen** (a Firefox/Gecko fork) - and Zen has two structural weaknesses our engine choice erases:

| Zen's fatal flaw | Why it hurts | How CEF/Chromium fixes it for us |
|---|---|---|
| **Gecko RAM footprint** | Users report loving the UX, then switching back "after a week because of RAM." | Chromium's memory profile + a disciplined Rust core and tab-suspension policy. |
| **No Widevine / DRM** | Netflix, Spotify, Disney+, HBO, Prime all break - a daily-use dealbreaker for many. | Chromium ships the Widevine plumbing (licensing caveats in §3.2), and site-compat is Chromium-grade. |
| **CSS-on-Firefox chrome** | Zen fakes native feel by injecting CSS into Firefox's UI. | We render **real native OS chrome** (SwiftUI + Liquid Glass) - the thing the design crowd actually feels. |

The "power-user maximalist" lane (Vivaldi) and the "AI-native" lane (Comet/Dia/Atlas) are crowded and
VC-funded. The clean-native-OSS lane is comparatively empty. **That is our opening.**

### 1.2 The defensible triad

Three things, together, that no competitor holds all of:

1. **Native OS chrome** - real Liquid Glass, native controls, native animation. Beats Zen's CSS-on-Firefox and
   every Electron-chromed browser. This is the strongest *emotional* differentiator and the one design-lovers
   feel instantly.
2. **Rust core** - native request-path ad-blocking (Brave-style, impossible for MV3 extension blockers),
   low memory, and the anti-bloat story that Arc ("bloated") and Vivaldi (~960 MB / 10 tabs) can't tell.
3. **Open-source governance** - the post-Arc trust wedge. Arc's users were orphaned by a $610M Atlassian
   acquisition; the loudest reason people now pick Zen is *"it can't be killed."* Make fork-ability and
   transparent governance an explicit, load-bearing promise.

### 1.3 The three audiences and what actually wins each

| Audience | What makes them switch | Our answer |
|---|---|---|
| **Design / aesthetics** (Arc & Zen refugees) | Native-feeling chrome, minimalism, smooth animation, "can't be killed" | Native Liquid Glass chrome + OSS durability (§1.2) |
| **Power users** | Command palette, keyboard-first, vertical tabs + spaces, **low memory**, extensions | Command bar + spaces (MVP) → **Commands** in ⌘T + ⌘K palette/split/sessions (§5.3/§5.4); native ad-block *instead of* extensions (§3.1) |
| **Developers** | Real DevTools, CDP/automation, scriptability, OSS + hackable, MCP/agent surface | CDP is near-free (§3.5), userscript engine (§3.1), clean readable Rust codebase as a growth channel |

### 1.4 The one-line pitch

> **The Arc experience, native and open - fast because it's Rust, beautiful because it's real OS chrome, yours because it's open-source.**

### 1.5 AI posture (decided deliberately, see §3.5)

Do **not** enter the frontier agentic-AI race. Ship AI as **clean, safe table-stakes** (sidebar chat,
chat-with-tabs, an MCP surface devs can drive) and make **"local-first, transparent, trustworthy"** the
position - a lane no VC-funded AI browser can credibly claim while they're shipping unpatched prompt-injection
holes (§3.5, §8.4).

---

## 2. The competitive map (mid-2026) & the Arc lessons

### 2.1 Who holds what

| Browser | Engine | OSS? | Lane / killer feature | Status | What it means for us |
|---|---|---|---|---|---|
| **Arc** | Chromium | No | Vertical tabs + spaces (defined the category) | **Dead** - frozen May 2025; TBC acquired by Atlassian ($610M) | Our interaction north-star; also our cautionary tale (§2.2) |
| **Dia** | Chromium | No | AI "Skills"; chat-with-tabs | Active; Apple-Silicon-only; Pro $20/mo | Stripped Arc's power-UX to chase mainstream AI - leaves the power/clean lane open |
| **Zen** | Gecko | **Yes** (MPL-2.0) | Faithful Arc clone, free/OSS | Very active, ~41k★, donation-funded | **Closest competitor**; we beat it on RAM, DRM, native chrome |
| **Vivaldi** | Chromium | No | Maximal customization, tiling, sessions | Active, mature, ~$6.3M rev | The anti-model on complexity; mine its power features, not its overwhelm |
| **Orion** | WebKit | No | Chrome+Firefox extensions on WebKit | 1.0 Nov 2025; Kagi-funded | Their headline (extensions) is our hard problem (§3.1); their pain (partial APIs) we avoid |
| **Brave** | Chromium | Partial | Native Rust ad-block; privacy | 100M+ users | Proves native request-path ad-block is a real moat we can replicate |
| **Comet** (Perplexity) | Chromium | No | Agentic browsing | Free, ~18M MAU, $200M raised | Frontier AI; also the prompt-injection cautionary tale (§8.4) |
| **Atlas** (OpenAI) | Chromium | No | Agent Mode | macOS Oct 2025 | Ditto - merging into an OpenAI "superapp" |
| **Ladybird** | **Own engine** | Yes | Independent engine | Pre-alpha; nonprofit, big sponsors | Proves money+prestige flow to OSS browsers; also that from-scratch = 4+ yrs. Our CEF bet is the opposite (correct) call |
| **qutebrowser / Nyxt** | QtWebEngine / WebKit | Yes | Keyboard-driven, scriptable | Niche, devoted | The aspirational reference for our dev audience - cheap credibility |

### 2.2 The Arc post-mortem - six lessons baked into this roadmap

From Josh Miller's own "letter to Arc members" + TBC's internal data:

1. **The novelty tax is real.** "Too different, too many new things to learn, for too little reward." → **Keep
   the on-ramp Chrome-familiar; make novelty opt-in, revealed progressively** (§5.3 exit criteria).
2. **They shipped features nobody used.** Multiple Spaces = 5.5% of DAU; Live Folders 4.2%; calendar-hover 0.4%.
   → **Instrument feature usage from day one and kill anything under ~5% DAU** (§8.5). This requires a
   privacy-preserving analytics design (§8.3) - a real tension to resolve, not a free lever.
3. **Missing/late extensions drove churn.** → We *can't* ship Chrome extensions on CEF (§3.1) - so we must
   **native-absorb what people installed extensions for** and say so loudly.
4. **Performance is a feature, not a footnote.** Arc died partly on "bloated." → **Make lightness a headline
   claim, defended with published benchmarks** (§8.2).
5. **OSS is the structural moat against the Arc death.** → Lead with fork-ability + governance (§7.2).
6. **Pick ONE legible differentiator.** Arc had many diffuse novelties and no single "this is why." →
   Ours is §1.4. Every winner has one line: Brave=ad-block, Orion=extensions, Zen=free-OSS-Arc, Dia=Skills.

---

## 3. Strategic decisions to lock (the forks)

These are the calls that reshape everything downstream. Each is framed as **tension → options → recommendation
→ your call.** Several revisit `architecture.md` - flagged where they do.

### 3.1 Extensions - the biggest fork ⚠️ (revisits spec §2, "extensions not required")

**The tension.** The Arc post-mortem says *lack of extensions drove churn* - for power users and developers,
extensions are historically the #1 switch-blocker (Orion's entire pitch). But the CEF research is unambiguous:
**real Chrome-extension support is effectively out of reach on CEF, and largely gone from Chromium itself.**

- CEF's old native extension API (`LoadExtension` / `CefExtensionHandler`) was **deprecated and removed with
  the Alloy bootstrap in M128.** The replacement is an *"unsafe"-flagged* CDP loader that **only works with
  Chrome-*style* windows showing real Chrome toolbar UI** - the opposite of our chromeless native-chrome shell.
- CEF exposes **~4 of Chromium's 70+ extension APIs**; the enabling roadmap issue (chromiumembedded/cef #3450)
  has sat unimplemented since 2023.
- **Even a perfect host wouldn't bring back full uBlock Origin:** Manifest V3 killed blocking `webRequest`
  upstream in Chromium; only uBO Lite (rule-capped, no cosmetic filtering) survives *anywhere* on current
  Chromium. This isn't a CEF gap - it's gone from the platform.
- The browsers that *do* ship extensions (Brave/Edge/Vivaldi) are **full Chromium forks**, not CEF embedders -
  fork-level investment we explicitly rejected by choosing CEF.

**This reframes the decision** from "extensions vs. no extensions" to *"how do we native-absorb what people
installed extensions for?"* - which turns out cleaner, safer, and lighter, and fits the thesis.

**Options:**

| Option | What it is | Cost | Verdict |
|---|---|---|---|
| **A - Native-absorb + userscripts** (recommend) | Build the top extension needs as native features: request-path ad-block, native password vault, native dark/reader mode. Ship a **Tampermonkey-class userscript engine** (feasible via CDP `Page.addScriptToEvaluateOnNewDocument` - the research confirms this is production-grade). Position "no extension bloat" as a virtue. | Medium; mostly work we're already doing | **Best fit.** Covers ~90% of real extension use with a cleaner/safer story. |
| **B - Fork Chromium for real extensions** | Abandon CEF, maintain a Chromium fork like Brave. | Enormous - the exact cost CEF was chosen to avoid; full-time Chromium-rebase role | Reject. Kills the whole architecture. |
| **C - Curated extension allowlist** | Load a vetted set of known-good extensions. | Blocked by the unimplemented #3450 + the Chrome-UI gate | Not viable today. |

**Recommendation: Option A.** Make the native ad-blocker excellent (it's already spec'd, §11), add a native
password vault (§10) and native dark/reader mode, and ship the **Tweaks** engine - per-site page customization (Snip element-remover /
custom CSS / userscripts; spec §20) - for the power/dev crowd who want programmability. Tweaks is the
native-absorb answer to uBlock's element picker, Stylus, Tampermonkey, and Arc Boosts, and reuses the adblock
cosmetic-injection path (§11). Be explicit in marketing: *"no extensions - because the things you needed them for are built
in, native, and can't be bought by an ad-tech acquirer."* Revisit only if you ever decide to fork Chromium.

**→ Your call:** confirm we abandon general extension support and commit to native-absorb + userscripts.

### 3.2 DRM / Widevine & codecs - a labeled limitation, not a launch feature ⚠️

**The reality.** The Widevine CDM *binary* is easy (CEF's component updater fetches Google's blob). The **license
is the wall**: it requires a discretionary agreement from Google + VMP signing, historically **denied to
open-source indie browsers** - the documented precedent is a dev told verbatim *"we're not supporting an open
source solution like this,"* after a 4-month wait (Samuel Maddock/Metastream; corroborated by The Register).
Even *with* a license, desktop browsers get **L3 → SD/720p only** (1080p/4K needs hardware L1 desktop can't
provide). Vivaldi/Brave ship Google's CDM because they're incorporated companies Google will contract with.

**Codecs are a separate landmine.** H.264/AAC/HEVC are patent-encumbered independent of copyright license.
Chromium ships them *off* by default; enabling `ffmpeg_branding=Chrome` means **distributing software decoders
= real patent royalties** (Via LA for H.264/AAC; the worse, multi-pool, rising-in-2026 HEVC).

**Options for DRM:** (a) ship L3 EME plumbing, don't promise Netflix; (b) later, engage a paid CWIP/3PL partner
(castLabs) for a license + VMP signing; (c) incorporate and pursue a Google license directly.

**Recommendation:**
- **DRM:** Ship L3 EME plumbing (near-free via CEF). **Do not advertise Netflix/Disney+ at launch** - document
  it as a known limitation. Budget a **post-1.0 castLabs/CWIP workstream** to secure a license + VMP signing
  (also requires the signed bundle we need anyway). This still *beats Zen*, which has no DRM at all.
- **Codecs - RESOLVED 2026-07-09 (ADR-0040), revising the earlier OS-decoder recommendation:** we build a
  **custom CEF with `proprietary_codecs=true ffmpeg_branding=Chrome`** on the pinned line (ADR-0028), via a
  scripted ephemeral cloud-Mac build (~€10/build, ~€12 per CEF bump), and point `cef-dll-sys` at it. Research
  confirmed **no usable prebuilt exists** (OBS's is the only codec-enabled macOS arm64 prebuilt - CEF 127,
  ABI-incompatible) and the OS-decoder path has a known-unsolved AAC problem (cef#3559). Patent posture,
  accepted deliberately: AVC is royalty-free under Via LA's 100k-units/yr tier; **AAC has no free tier** -
  exposure carried knowingly at pre-launch scale, revisit before distribution numbers matter. **HEVC stays
  out of the bundle** (OS or absent) - that part of the old recommendation stands. Lean on AV1/VP9 where
  sites offer them.
  - **Distribution posture - DECIDED 2026-07-09 (ADR-0042, supplements 0040):** public GitHub-Actions browser
    releases **ship the codecs** (a conscious pre-launch risk - the app makes them public regardless of where
    the CEF artifact is stored; the private `ochre_cef` repo is upstream-hygiene, not patent cover). Maintainer
    release workflow (read-only PAT for `ochre_cef` in repo secrets) builds codec-full; fork-PR CI can't reach the secret so
    contributor builds fall back to the stock codec-less prebuilt - a free **maintainer-codecs /
    contributor-codecless** split. Codec-less is the **one-flag retreat**, kept working, for when scale or
    visibility makes the AAC exposure uncomfortable.

**→ Your call:** ~~accept "no premium DRM at launch, labeled clearly" and the OS-decoder codec posture.~~
DRM half stands as recommended (labeled limitation); codec half **decided** → ADR-0040.

### 3.3 The CEF release lane - Stable vs. LTS

**The tension.** A browser is a primary attack surface. Chrome patches ~weekly, emergency-patches ~8 exploited
zero-days/year, and **moves to a 2-week release cycle on Sept 8, 2026** - roughly doubling rebuild cadence. CEF
tracks Chromium per-milestone but the branch-merge is **manual, with a delay**; you inherit that lag.

**Recommendation:** Anchor to the **CEF LTS branch** (every 6th milestone, ~8 months of security fixes) as the
default lane - bump 2–3×/year - **with a documented ability to hot-jump to a Stable respin for any KEV-listed /
actively-exploited 0-day.** This caps routine churn for a small team while preserving an emergency path. Budget
the treadmill as a **standing release-engineering cost** (~30–60k tokens per bump for API-delta fixes +
regression), not a one-time task (§8.1).

**→ Your call:** confirm LTS-default + Stable-hot-jump.

### 3.4 Sync - build it yourself, design for it early ⚠️ (spec §12 defers it; the market says don't defer the *design*)

**The reality.** Chrome Sync is a proprietary Google service - **absent from CEF, cannot be added.** Yet
"no sync / no cross-device" is a top recurring Arc and Zen complaint; power users assume it. You must build
your own account + E2E-encrypted sync layer (tabs/spaces/bookmarks/settings/vault).

**Recommendation:** Keep sync *implementation* out of MVP (spec §12 is right that it's the highest-risk external
dependency), **but design the tab/space/session/vault models to be sync-ready from the start** so you're not
retrofitting. Ship **sync v1 in Phase 3 (1.0)** - it's part of what makes this a *daily* browser, not a second
browser. Follow Zen's honest posture (E2E, server can't read it). A companion mobile app (the actual #1 "why I
left" theme) is the eventual sync target (§3.6).

**→ Your call:** confirm sync is a 1.0 pillar (design-early, ship Phase 3), not a "someday."

### 3.5 AI / agent layer - clean table-stakes, CDP-first, safety as the position (spec §19)

**The reality (from the AI + MCP research):**
- **Genuinely useful, now table-stakes:** sidebar chat-with-page, **chat-with-tabs / cross-tab query** (the one
  "only a browser can do this" feature), and **tab/download auto-organization** (Arc's underrated Tidy-* - the
  industry abandoned it chasing agents).
- **The dev-differentiated layer:** **MCP** (browser-as-tool for agents) + **local models / BYOM** (Ollama,
  à la Brave Leo - traffic never leaves the device).
- **Mostly gimmick + unsafe today:** autonomous agent-mode on authenticated sessions. Verified, *unpatched*
  prompt-injection hijacks against both Comet and Atlas; OpenAI concedes it may never be fully solved.

**The near-free technical path:** CEF speaks the full **Chrome DevTools Protocol** via
`--remote-debugging-port` *and* in-process (`ExecuteDevToolsMethod`, no port/DevTools window needed). Turning on
a loopback-bound, consent-gated CDP port instantly makes the browser drivable by Playwright/Chrome-DevTools-MCP/
browser-use with **zero agent-side code.** A native, semantic MCP server (exposing our spaces/vault/adblock, not
raw CDP) is a thin later layer on top.

**Recommendation:**
- **Phase 3:** ship the **CDP automation surface** (loopback, flag/consent-gated) - cheap, high dev value.
- **Phase 4:** sidebar assistant + chat-with-tabs + auto-organization; **agentic create/edit of Commands**
  in-app (natural language → TypeScript in the manager, same consent gates - ADR-0034); **MCP host/client**
  and **BYOM/local models** as the dev-facing differentiators; a native semantic MCP server.
- **Safety is the product position:** never run agents on authenticated sessions without human-in-the-loop;
  adopt an approved-sites model for any autonomous action; treat prompt-injection defense as first-class (§8.4).
  Market **"local-first, transparent, yours."**

**→ Your call:** confirm AI = clean table-stakes + dev MCP/local-models + safety-as-position (not the frontier race).

### 3.6 Mobile - out of desktop scope, but the honest #1 churn reason

No-mobile / no-companion is the single most-cited "why I left Arc/Zen" theme. A mobile browser is a separate,
large project (and iOS forces WebKit). **Recommendation:** explicitly out of scope through 1.0; set expectations
publicly; treat a **thin mobile companion (sync target + read/tabs, not a full engine)** as the first Phase-4+
expansion once desktop sync exists. **→ Your call:** confirm mobile is post-1.0.

---

## 4. Feature taxonomy - the table-stakes gap-check

The full set a mainstream user unconsciously expects, tiered by *how fast the absence is noticed*. ⚙️ = mostly
free from CEF/Chromium (wire the UI); 🔨 = you build it even though the engine is Chromium.

**Tier 0 - "not a real browser" (blocks daily-drive day one):** omnibox + nav + search ⚙️ · autocomplete/
suggestions 🔨 · new/close/**reopen-closed-tab** 🔨 · **session restore** 🔨 · **crash recovery** 🔨 · tab titles+
favicons ⚙️ · HTTPS + cert-error UI ⚙️ · cookies/login persistence ⚙️ · **downloads to disk** 🔨 · file upload
picker ⚙️ · **find-in-page** 🔨 · right-click menu 🔨 · clipboard in content ⚙️ · **keyboard/focus routing** 🔨
*(spec §15 #1 risk)* · **set-as-default + open links from other apps** 🔨 · permission prompts (cam/mic/loc/
notif) ⚙️.

**Tier 1 - noticed within days:** **import from Chrome/Safari/Arc** 🔨 *(the switching-cost killer)* ·
bookmarks 🔨 · history 🔨 · **downloads manager** 🔨 · **password save+autofill** 🔨/⚙️ · OTP+passkey UX 🔨 ·
form/address/payment autofill ⚙️ · zoom ⚙️ · **PDF inline** 🔨 *(verify CEF ships pdfium)* · print / Save-as-PDF
🔨 · fullscreen (browser+video) ⚙️ · background audio + per-tab mute ⚙️ · web push 🔨 *(gated - §4.2)* ·
**native ad/tracker block** 🔨 *(the #1 extension need)* · **settings UI** 🔨 · clear
browsing data ⚙️ · private window ⚙️ · baseline shortcuts 🔨.

**Tier 2 - missed over weeks:** reader mode 🔨 · translate 🔨 · spellcheck ⚙️ · picture-in-picture ⚙️ · save
page 🔨 · per-site permission management 🔨 · cookie/site-data controls 🔨 · custom search + bang keywords 🔨 ·
profiles/containers 🔨 · **command palette / tab search** 🔨 · tab hover previews 🔨 · accessibility (native,
free) ⚙️ · Safe Browsing ⚙️ *(verify enabled in CEF)* · media-key/Now-Playing ⚙️ · drag-drop (links/tabs) 🔨.

**Tier 3 - differentiators / long-tail:** split view · notes/capture · **Tweaks** (Snip / per-site CSS / userscripts, §20) · mouse gestures ·
theming/Mods · PWA/site-as-app · **vertical tabs + spaces** *(Tier-3 for a generic browser, but our core thesis
→ effectively Tier 0 for us)*.

**Operationally-Tier-0 even though users never "see" them:** **auto-update** (can't ship security patches
without it) and **signed+notarized builds** (gate passkeys, Sparkle, Gatekeeper).

### 4.1 Minimum Viable Daily Driver

The smallest set below which nobody switches = **all of Tier 0** + from Tier 1: import, bookmarks, history,
downloads manager, password save+autofill+OTP/passkeys, address/card autofill, zoom, PDF inline, print/
Save-as-PDF, fullscreen, background-audio+mute, web push, private window, settings UI, clear-data, native
ad-block, baseline shortcuts - **plus** auto-update and proven CEF↔native focus routing. This set
*is* the Phase 2 bar (§5.3).

### 4.2 CEF is not Chrome - the web-platform gaps that gate "a complete browser"

The prebuilt CEF binary (what the `cef` crate downloads) is **not** feature-equivalent to Chrome. Several of
these directly hit the Phase-2 "complete browser" claim and are non-obvious. Good news first: **CEF's Chrome
runtime is the default since M128**, so camera/mic/geolocation *permission prompts* now behave like Chrome
out of the box (the old Alloy "silently deny" DX is gone). The real gaps:

- **Proprietary codecs (H.264 / H.265 / AAC) are OFF in the prebuilt binary** - patent reasons. Stock CEF
  **cannot play most MP4 / a lot of web video**. **Decided 2026-07-09 (ADR-0040):** fix (a) - **build CEF from
  source** with `proprietary_codecs=true ffmpeg_branding=Chrome` on the pinned line, via a scripted cloud-Mac
  build (backlog P2-38). Fix (b), OS-provided decoders, was rejected: AAC can't be cleanly routed through OS
  decode APIs (the finding that stalled cef#3559). AV1/VP9 (YouTube's usual path) are royalty-free and work.
- **Web Push / FCM is effectively unsupported** on stock CEF (needs Google's private API keys, only compiled in
  from a source build). So Slack-web/calendar notifications don't arrive → this is 🔨, not free.
- **The Google-API-key cluster is off:** Safe Browsing, network **geolocation**, Google-backed spellcheck/
  translate/suggestions, and Chrome Sync all require API keys the prebuilt lacks. Safe Browsing being off is a
  **trust/security gap to close** (inject your own keys via a source build, or ship an alternative).
- **WebGPU is broken out of the box** on Windows (a flag + two missing DLLs, `dxil.dll`/`dxcompiler.dll`) - a
  dev-audience gap worth fixing since we court developers.
- **WebUSB / WebSerial / WebHID / WebBluetooth are non-functional** - the APIs exist but CEF surfaces **no
  device-chooser UI**, so `requestDevice()` fails. Niche, but a hard gap for hardware-adjacent dev sites.
- **PDF plugin** is frequently absent from CEF prebuilts - verify before assuming inline PDF.
- **Translate, reader mode, DoH, Chromium's own password manager** live in Chromium's `chrome` layer and are
  each their own custom project in CEF. **Chrome Sync is gone** - design your own (§3.4).
- **Passkeys need a signed + entitled bundle** - currently `CODE_SIGNING_ALLOWED: NO` in `project.yml`; the
  entitlements are also Apple-request-gated (§6.1). Flip signing early.

**Net:** budget a **"CEF web-platform parity" workstream in Phase 2** - most likely a **custom CEF build** (for
codecs + API keys) rather than the stock prebuilt. This is the single most underestimated item in the plan;
decide the build-vs-prebuilt question before Phase 2 scope locks.

---

## 5. The phased roadmap

Five phases. Each: **goal · scope · exit criteria · key risks.** Effort is expressed in estimated tokens
(agentic sizing), not calendar time. Phases 0–1 reference the existing docs rather than duplicating them.

> **Phases are the *public-product bar*, not the *frontend schedule*.** The five phases below track
> "how complete/releasable is the product" and are written from the macOS lead frontend. **Platform
> expansion is an orthogonal track:** because ~80% of behavior is in the shared Rust core, a second
> frontend is mostly native view + binding, gated only on the core (done) - not on any later phase.
> **Windows is now starting as a parallel frontend track** (README: "Windows is next"), building
> against `parity.md`'s Win column as macOS MVP1 completes - *not* deferred behind the Phase-4 AI
> work it was originally filed under (§5.4). So read Phase 2/3 as the macOS-lead product bar with a
> concurrent Windows build catching up to parity beneath it; the two converge, they aren't
> sequential. Sequencing detail: `spec §8,§16`.

### 5.0 Phase 0 - Foundation (de-risk the thesis) · *mostly done*

**Goal:** prove the whole bet physically works on a signed build.
**Scope:** the spike (`spike.md`) is structurally validated; the missing entry points now exist and the
workspace compiles. Remaining to *close* Phase 0:
- Prove **CEF `NSView` ↔ SwiftUI keyboard/focus routing** (spec §15 #1 - the highest risk; page typing vs Cmd+T
  vs sidebar focus, click-to-focus transitions).
- Flip to a **signed + hardened-runtime bundle** and confirm a **passkey/Touch-ID login** works (gated on
  signing per §6.1).
- **Native ad-block on the CEF request path** (`CefResourceRequestHandler` + `adblock` crate) - cheap once the
  hook exists, and it's a headline differentiator.
- Turn on the **CDP surface** (loopback, gated) - near-free, unlocks the dev/agent story later.

**Exit:** a single signed tab that renders real sites, opens DevTools, completes a passkey login, and blocks on
the request path. **Risk:** focus routing is the one that can force a design change - resolve before building UI on it.

### 5.1 Phase 1 - MVP1: "the thesis, daily-drivable by *you*" · = `mvp1.md`

**Goal:** the Arc interaction model in native chrome, good enough to dogfood. **Not public.**
**Scope:** exactly `mvp1.md` M1–M5 - multi-tab CEF in `NSView`s, vertical-tab glass sidebar, spaces,
Cmd+T command bar with the Liquid Glass morph, slim content-first chrome, native ad-block, core shortcuts.
**Exit:** *you* daily-drive it on macOS. **Risk:** the command-bar glass morph fidelity (§ spec 15 #2) and
per-tab CEF memory policy.

*Everything from here on is new territory this roadmap defines.*

### 5.2 Phase 2 - Public Beta: "a complete browser" · **the first public release**

**Goal:** a stranger can switch without hitting a wall in week one - the **Minimum Viable Daily Driver** (§4.1)
- and you can responsibly ship security patches. This is the "credible, releasable" bar the whole project aims at.

**Scope - table-stakes fill (Tier 0 gaps + critical Tier 1):**
- **Trust/switching:** import from Chrome/Safari/Arc (bookmarks, passwords, history); set-as-default + OS link
  handling; session restore + crash recovery + reopen-closed-tab.
- **Core surfaces:** history, bookmarks, **downloads manager**, find-in-page, settings UI,
  private window, clear browsing data, per-site permission prompts, zoom, fullscreen, background-audio+mute,
  web push.
- **Content:** inline PDF (verify pdfium), print / Save-as-PDF.
- **Credentials:** native **password vault** + autofill - fork **resolved to a Bitwarden-compatible,
  rbw-derived client** (spec §10; ADR-0016), pulled forward and built core-crypto-first; plus OTP +
  **passkey** prompts (needs the signed bundle from Phase 0). Autofill's bar is **real-world robustness,
  not just the static form present at load**: SPA / dynamically-injected forms, **two-step email-first
  flows** (Google/Microsoft/Okta), shadow-DOM-hosted forms, **TOTP / one-time-code** fill, framework-input
  fill fidelity, and **save/update capture** (backlog P2-28…P2-33) - plus a **URL-bar quick-access button**
  (Fill/Copy for the current site), the detection-independent manual path where Copy always works (P2-34, done;
  ADR-0029). Two situations stay gated: cross-origin **iframe** logins ("Sign in with X" embeds) need the
  render-process path (P2-24, ADR-0027), and the native **save-password banner** needs the M2 chrome (P2-23).
- **Native-absorb (§3.1):** ad/tracker block matured; native dark/reader mode; **Tweaks** - Snip (element
  remover) + per-site CSS, with the userscript (Script) tier in beta (spec §20).

**Scope - release infrastructure (operationally Tier 0, §6):**
- **Signed + notarized + hardened-runtime** builds (flip `CODE_SIGNING_ALLOWED`).
- **Sparkle 2.x auto-update** with EdDSA + delta pipeline in CI (deltas are critical for a ~200 MB CEF bundle).
- **Opt-in, self-hosted crash reporting** (Crashpad → GlitchTip/Sentry, minidumps scrubbed) + a **CVE-watch +
  one-command respin** pipeline (§8.1).
- **Chromium phone-home audit** - neutralize Google's default variations/component-update/Safe-Browsing/
  translate endpoints; publish the surviving endpoint list (§8.3).
- Website, download page, **privacy policy** (required the moment anything leaves the machine), a Discord +
  public GitHub.

**Exit:** MVDD complete; signed/notarized/auto-updating; a handful of external beta users daily-drive it without
a blocking gap. **Risks:** the password-vault build; import fidelity; and the moment you're public you own the
**security treadmill** forever (§8.1).

### 5.3 Phase 3 - 1.0: "the power-user & developer browser" · **the credible public 1.0**

**Goal:** the differentiators that create the "I can't go back" effect, plus the non-feature bar that makes a
1.0 credible. This is where we win the power/dev audiences, not just avoid losing the mainstream one.

**Scope - power-user toolbox** (ranked by "disproportionately wins the audience"):
1. **Commands** (ADR-0034) - user-defined browser automation (Glide-class: tab/process/path APIs, TypeScript,
   vibe-coded). **Primary surface: the ⌘T command bar** - URLs, tabs, history, search, *and* Commands in one
   unified ranked list; new-tab navigation stays first-class. **Secondary surface: a ⌘K command palette** biased
   toward actions (built-ins + the same Command registry; no URL/search navigation). Settings manager + cookbook.
2. **Keyboard-first navigation** - customizable shortcuts + link-hint ("vim") mode; optional per-Command key
   bindings; the qutebrowser credibility play.
3. **Tab search / switcher** + **tab suspension** (native, Vivaldi-granular per-space/stack hibernation).
4. **Split view** - grid + workspace tiling (Vivaldi-depth, since a 2-pane split is now Firefox-baseline).
5. **Named session management** (save/restore, auto-backup - the genuine Vivaldi moat).
6. **Per-site settings/zoom**, **custom search engines + bang keywords**, **profiles/containers**.
7. **Web panels / sidebar apps** (Vivaldi), **focus/reader mode**, **screenshot/capture** - an internal
   capture system: window capture, full-page capture with region crop/select, and video/screen recording -
   PWA/site-as-app.

**Scope - developer toolbox** (mostly CDP-powered, near-free per the dev-feature research):
- **DevTools** docked (not just popup) + a clean **CDP/automation surface** (loopback, consent-gated).
- **UA switching + device/responsive emulation** (CDP `Emulation.*`), **network inspection + throttling +
  request mocking** (CDP `Network`/`Fetch`), and a **remembered per-origin localhost self-signed-cert trust**
  (native `OnCertificateError`) - a concrete point of *superiority* over Chrome's regressed cert flow.
- **Tweaks** (GA) - the page-customization surface: Snip (element remover), per-site CSS (Restyle), and the
  **userscript engine** (Script tier) + custom CSS/JS injection (spec §20).
- **Built-in visual inspect/design tools** - a lightweight layer *beside* full DevTools: an **eyedropper**
  color picker (native OS sampler), a **measure/guides** ruler (element-to-element distances), **element peek**
  (box-model + styles on hover), a **font inspector** (rendered family/metrics + `@font-face` source), a
  **palette/type extractor**, and a small **encoder scratchpad** (base64/JWT/URL/timestamp/regex/JSON
  pretty-print) + **QR-for-URL** hand-off. Split across two mechanisms already built - native overlays
  (eyedropper, capture) and the document-start injection substrate (ADR-0017, the DOM-aware tools). The
  native-absorb answer to the DevTools-adjacent extensions (ColorZilla, page rulers, font inspectors, VisBug)
  - backlog P3-1…P3-12.

**Scope - the 1.0 bar (non-feature):** sync v1 (E2E, §3.4); stability hardening; docs; a real community;
a funding channel live (§7); reproducible-builds as a stated goal (§8.3).

> **Architectural leverage - build one primitive, satisfy three audiences.** Spaces (design/power-user),
> developer containers (isolated logged-in/out test contexts), and per-context AI identity are the *same*
> underlying thing: a per-context CEF `RequestContext` + cache path + credential set. Build that isolation
> primitive once in the Rust core (design it in from M1, per `mvp1.md`'s space-aware tab model) and the
> power-user, developer, and AI features on top of it all become cheap. Prioritize it - it's the backbone.
> (Corollary of the native-chrome bet: you build every piece of *browser UI* yourself, but DevTools, CDP, and
> the web platform come from CEF for free - plan effort accordingly.)

**Exit:** a credible public 1.0 that a power user or developer chooses *over* Chrome/Arc-successors for the
native feel + speed + hackability + OSS trust. **Risks:** scope discipline (§9) - this phase is where the Arc
"maximalist novelty tax" trap lives; instrument and prune (§8.5).

### 5.4 Phase 4 - Top-tier: AI, cross-platform, depth · *ongoing*

**Goal:** genuine top-tier, cross-platform, with the AI layer done cleanly and safely.
**Scope:**
- **AI (§3.5):** sidebar assistant + chat-with-tabs + tab/download auto-organization; **agentic create/edit
  of Commands** (and Tweaks) in-app - propose TypeScript/CSS, user reviews and enables; **MCP host/client** +
  **BYOM/local models** (Ollama); a native semantic MCP server exposing our spaces/vault/adblock/Commands;
  safe-by-design agentic actions (approved-sites, human-in-loop, prompt-injection defenses).
- **Windows** (spec §8) - `windows-reactor`/WinUI mirror (Mica/Fluent), keeping the app all-Rust if the reactor holds. **Pulled forward to a parallel track** (see the §5 callout): it's started as soon as the core is stable and builds against `parity.md`'s Win column concurrently with Phase 2/3 - it is *not* gated on reaching Phase 4. Listed here only because it's cross-platform *depth*; the schedule is parallel, not Phase-4-sequential.
- **Mobile companion** (§3.6) as a sync target.
- **Depth:** theming/Mods, mouse gestures, notes/annotations, advanced spaces - each gated on usage data.
- **DRM:** the castLabs/CWIP Widevine workstream (§3.2), if pursued.

**Exit:** cross-platform, AI-capable, and defensibly top-tier - while never losing the lightness/native/OSS
identity. **Risk:** this is forever; the security treadmill and performance defense are permanent line items.

### 5.5 One-screen phase summary

| Phase | Name | Public? | The bar it clears |
|---|---|---|---|
| **0** | Foundation | No | The thesis physically works on a signed build |
| **1** | MVP1 (= `mvp1.md`) | No (dogfood) | *You* can daily-drive the Arc-in-native-chrome UX |
| **2** | Public Beta - "a complete browser" | **Yes** | A stranger switches without a week-one wall; you can ship patches |
| **3** | 1.0 - "power-user & dev browser" | **Yes** | Power users/devs choose it over the alternatives |
| **4** | Top-tier - AI, cross-platform | Yes | Genuinely top-tier, cross-platform, AI-clean |

---

## 6. Release, distribution & the 1.0 bar

### 6.1 macOS release requirements

- **Apple Developer Program** ($99/yr) → **Developer ID** signing + **notarization** + **hardened runtime** +
  stapling. Flip the current `CODE_SIGNING_ALLOWED: NO`.
- **CEF-specific signing pain (the footguns):** sign **bottom-up** (nested helper execs → each Helper `.app` →
  CEF framework → main exec → outer `.app`); **never `codesign --deep`** (it mis-signs the framework tree and
  notarization rejects it); package with **`ditto -c -k --keepParent`, never `zip`** (zip mangles the
  signature). Ship a **DMG** (drag-to-`/Applications`) to avoid App Translocation breaking state persistence.
  Helper bundles need hardened-runtime entitlements: `com.apple.security.cs.allow-jit`,
  `allow-unsigned-executable-memory`, `disable-library-validation` (V8's JIT + loading our-team-signed CEF).
- **Passkey entitlements are Apple-request-gated** (you apply; not auto-granted): `com.apple.developer.web-browser`
  and `com.apple.developer.web-browser.public-key-credential` - the latter lets the app make WebAuthn requests
  for any relying party (how Chrome/Firefox do passkeys on macOS via iCloud Keychain). These **require** the
  signed+entitled bundle - the gate on a whole slice of daily use (banking, 2FA).
- **Mac App Store is effectively impossible** for a CEF browser (App Sandbox vs. multi-process helpers +
  entitlements). Ship **Developer-ID + notarized**, not MAS.

### 6.2 Auto-update (the safety-critical dependency)

- **Sparkle 2.x** (MIT) - appcast + **EdDSA** signing + **binary deltas** (`generate_appcast` in CI). Deltas
  matter enormously: a Chromium security respin changes a fraction of a ~200 MB bundle; without deltas users
  abandon the download and stay exposed.
- Store the Ed25519 **private key in CI secrets**, not just a dev Keychain (bus-factor).
- Cross-platform later: **WinSparkle** (Windows, same appcast model) → Squirrel/Omaha at scale; **Flatpak**
  (Linux, auto-update built in) + AppImage/zsync.

### 6.3 The security treadmill as a first-class system (§8.1 expands)

CVE-watch job (Chrome Releases feed + **CISA KEV**) → one-command **rebuild-against-CEF → sign → notarize →
gen-delta appcast → publish** → an **emergency-respin runbook** with a **days-not-weeks SLA** for exploited
0-days. Anchor to **CEF LTS + Stable hot-jump** (§3.3). Chrome's **2-week cadence from Sept 2026** makes this
non-optional.

### 6.4 Distribution channels

Direct **DMG** download + **Homebrew cask** at launch. Later: Windows (winget, **EV cert** to avoid SmartScreen
warnings), Linux (Flatpak first). No Mac App Store (§6.1).

### 6.5 The "1.0 bar" checklist (beyond features)

Signed+notarized builds · working auto-update with deltas · crash-recovery + stability · a website + download
page + docs · a **privacy policy** · a community (Discord + GitHub) · a published telemetry/"what leaves the
machine" stance · a live funding channel.

---

## 7. Sustainability & governance

### 7.1 Funding - the realistic ladder for a small OSS browser

No indie browser monetizes the browser *directly* at meaningful scale early. The teardown of Zen/Vivaldi/Brave/
Orion/Ladybird/Firefox yields this ladder, roughly in order of accessibility:

1. **Founder runway** - assume it; it funds MVP → traction. (Every browser here ran on it first.)
2. **Donations day one** - GitHub Sponsors + OpenCollective/Patreon/Ko-fi. Transparent finances build trust
   (Zen/Ladybird lean on this). Realistic early scale: hundreds–low-thousands $/mo (Zen ≈ 49 GitHub sponsors).
3. **Search affiliate / partner referral** - turn on as soon as you have users; tiny per-user yield.
4. **Optional "pro" tier** (Orion+ shape, ~$5/mo) once you have a loyal base; free stays fully functional.
5. **Corporate sponsorship / grants** (Ladybird model; NLnet/Sovereign Tech Fund) - relationship-gated, lumpy.
6. **Search-default revenue-share** - *the graduation milestone*. Google/Apple are unreachable, but the
   **privacy/aggregator engines (DuckDuckGo, Ecosia, Qwant, Startpage) run partner rev-share programs reachable
   at far smaller scale** - this is Vivaldi's escape hatch and the bridge from donations to a business.
7. **Parent-business subsidy** (Kagi/Orion) - the most durable long-term model *if* the browser anchors a future
   paid product (search, sync, AI). Worth designing toward.

**Avoid** the Brave ad-tech/crypto model - high-trust-cost, repeatedly damaged their reputation, and off-limits
at indie scale. **Mozilla is the cautionary ceiling:** even the #2 org is ~85% dependent on one antitrust-
threatened Google contract. **Do not architect around getting a Google deal.**

### 7.2 Governance - the anti-Arc promise

OSS is table-stakes, not sufficient. The trust ladder: **open source (now) → a plain-English "what leaves the
machine" doc (Phase 2) → third-party security audit (post-traction) → reproducible builds (stretch goal).**
Codec patents + the proprietary Widevine blob are the two edges that keep a *fully-featured binary* from being
"clean OSS" (§3.2) - the *source* can be fully open; disclose the binary's proprietary components.

Make **fork-ability + transparent governance** an explicit promise (the post-Arc wedge). Heed Ladybird's
June-2026 move to maintainer-only PRs as a scaling caution: **plan contribution governance before it's forced.**

---

## 8. Cross-cutting operational commitments

### 8.1 The security treadmill (permanent)

The single biggest ongoing tax of building on CEF. Budget it as standing release-engineering (not a chore):
LTS lane + Stable hot-jump (§3.3), CVE-watch automation, one-command respin, days-not-weeks emergency SLA,
~30–60k tokens/bump. Falling behind on a KEV-listed Chromium 0-day is the most likely way an indie browser makes
the news for the wrong reason.

### 8.2 Performance as a headline claim

The anti-Arc-bloat / anti-Zen-RAM position. Set a **memory + startup budget**, defend it with **published
benchmarks vs. Chrome/Arc/Zen/Vivaldi**, and make lightness a legible marketing claim - backed by the Rust core
+ native chrome + a real tab-suspension policy (§5.3).

### 8.3 Privacy / telemetry stance

**Ship zero behavioral telemetry at v1** (matches the newcomer/Zen expectation and is the cheapest trust to
earn). If you later need product analytics, adopt **privacy-preserving-by-construction** (Brave P3A/STAR-style
k-anonymity), **opt-in**, disableable - never opt-out raw telemetry (the durable Firefox/Vivaldi criticism).
**Audit and neutralize Chromium's default Google phone-home** (this is real engineering, not a checkbox) and
**publish the endpoint list.** Crash reports are opt-in, self-hosted, minidump-scrubbed (they can contain
passwords/URLs/cookies).

### 8.4 AI safety (if/when the AI layer ships)

Prompt injection is the category's unsolved Achilles heel (verified hijacks on Comet + Atlas). Rules: **never run
agents on authenticated sessions without human-in-the-loop**; approved-sites model for autonomous actions;
consent + injection defense as first-class; local-first/BYOM as both a privacy feature and a smaller attack
surface. This *is* the differentiation vs. the VC-funded AI browsers.

### 8.5 Feature-usage instrumentation (the Arc lesson) - with a tension to resolve

Arc built elaborate features almost nobody used and couldn't see it until the DAU data arrived. **Instrument
adoption and prune anything under ~5% DAU.** This directly tensions §8.3 (zero-telemetry) - resolve it by making
usage analytics **local-first and privacy-preserving** (aggregate/k-anonymous, opt-in), so you learn what to
kill *without* becoming the surveillance default you're positioning against.

---

## 9. What to deliberately NOT build (scope discipline)

The twin failure modes are **Arc's novelty tax** (clever features nobody uses) and **Vivaldi's overwhelm**
(everything, un-hidden). Guardrails:

- **No general Chrome-extension host** (§3.1) - native-absorb + userscripts instead.
- **No premium-DRM promise at launch** (§3.2) - labeled limitation.
- **No kitchen-sink extras** (built-in email/calendar/VPN/RSS à la Vivaldi/Opera) unless usage data earns them.
- **No frontier autonomous agents** (§3.5) - clean table-stakes + safety instead.
- **No feature without a legible reason** - every UI element is a tax; three worn paths beat thirty clever ones.
- **Keep the on-ramp Chrome-familiar** - novelty is opt-in, revealed progressively (the Arc lesson).
- **One legible differentiator** (§1.4), defended relentlessly.

---

## 10. Open decisions for you (the §3 forks, collected)

| # | Decision | Recommendation | Revisits spec? |
|---|---|---|---|
| 3.1 | **Extensions** | Abandon general extensions; native-absorb + userscript engine | Confirms spec §2 |
| 3.2 | **DRM / codecs** | DRM: no premium-DRM promise at launch (labeled). Codecs: **resolved → ADR-0040** - custom CEF build w/ proprietary codecs, no HEVC bundle | New |
| 3.3 | **CEF lane** | LTS default + Stable hot-jump for KEV 0-days | New |
| 3.4 | **Sync** | 1.0 pillar - design early, ship Phase 3, E2E | Sharpens spec §12 |
| 3.5 | **AI layer** | Clean table-stakes + dev MCP/local-models + safety-as-position | Sharpens spec §19 |
| 3.6 | **Mobile** | Post-1.0; thin companion as sync target | New |

Once these are settled, the phase plan in §5 is executable as written.
