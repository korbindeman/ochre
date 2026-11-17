# Ochre

A minimal, fast, open-source browser with Arc's UX and real native OS apps.

Chromium via CEF, a shared Rust core, and a thin native UI per platform: SwiftUI on macOS, Windows Reactor (native WinUI 3 in Rust) on Windows.

**Early development.** The macOS MVP is complete and daily-driven; the Windows frontend is underway.

## Viability

Building a browser is hard, and harder for one person. So far CEF has exceeded my expectations: Chromium in a native shell, driven from a Rust core, with no rendering engine to maintain. There may be a ceiling (licensing walls, things CEF won't allow), and I plan to keep going until I hit it. Forking an engine like [Ladybird](https://ladybird.org/) is a possible long-term path, but that's past solo scale; for now CEF is the pragmatic choice.

## Features

> A snapshot; it may lag the repo.

Ochre is **scriptable and extensible**: a Rust core with a thin native shell and a consent-gated automation surface, so you can add your own workflows, AI included, without waiting on us to ship them.

- **Arc-style UX**: vertical tabs, spaces, pinned tabs & folders
- **Native adblock & cookie consent**: request-path blocking; reject or hide consent banners
- **Bitwarden / Vaultwarden vault**: unlock, sync, and autofill against your existing server
- **Proprietary media codecs**: H.264/AAC playback via a custom CEF build
- **Arc import**: migrate spaces, folders, and tabs from Arc's sidebar export
- **Commands** *(planned)*: user-scriptable automation in the command bar; TypeScript commands with access to tabs, spaces, navigation, clipboard, and more
- **AI** *(experimental)*: exploring what helps; no filler features. The scriptable surface is the point: wire in your own models and tools

**Not yet:**

- **Chrome extensions**: blocked by the engine. CEF removed its extension loader, and the replacement works only with Chrome's own toolbar UI, which a native shell can't host. Real extension support seems to mean forking all of Chromium. Commands and native features are the interim; extensions stay on the radar.
- **DRM / protected streaming**: blocked by licensing. Bundling Widevine is easy, but Netflix and similar services need a Google Widevine agreement and VMP signing, and Google has a track record of denying both to open-source indie browsers. On the list for later.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Native UI (per platform)                                   │
│  macOS: SwiftUI · Windows: WinUI 3 (Windows Reactor)        │
│  sidebar · spaces · command bar · hosts CEF child views     │
└───────────────▲──────────────────────────────┬──────────────┘
                │ FFI                          │ view host
┌───────────────┴────────────────┐   ┌─────────▼──────────────┐
│  Rust core                     │   │  CEF (Chromium)        │
│   tabs · spaces · session      │──▶│   one view per tab     │
│  adblock · automation          │   │  DevTools · CDP        │
└────────────────────────────────┘   └────────────────────────┘
```

The native app owns the window; CEF renders into embedded child views.

| Path | Contents |
|------|----------|
| `crates/core/` | Browser logic, CEF wrapper, tab/space model |
| `crates/ffi/` | C ABI bridge to native frontends |
| `apps/macos/` | SwiftUI macOS app |
| `docs/` | Architecture, roadmap, style guide, ADRs |

See [`docs/architecture.md`](docs/architecture.md) for the full design.

## Development

Development is agentic for the most part: coding agents implement features as end-to-end vertical slices (core, FFI, native UI, verification). Nothing ships on the agent's word alone; I review and dogfood every change, and the bar is a browser that holds up as a real daily driver.

[`CLAUDE.md`](CLAUDE.md) is the operating manual for agents: build commands, architecture constraints, CEF gotchas, and conventions. Point your agent at it and describe a concrete slice.

### Prerequisites

**All platforms**

| Tool | Why |
|------|-----|
| [Rust](https://rustup.rs/) (`cargo`) | Core, FFI, and bundle tooling |
| [`just`](https://github.com/casey/just) | Recipe runner, the project's command surface |
| `cmake` + `ninja` | CEF's `libcef_dll_wrapper` builds at compile time |
| Python 3 | Work-lease gating and a few helper scripts (`scripts/work-lease.py`), stdlib only |
| `bash` | Several `scripts/*.sh` entry points |
| Network (first build) | CEF prebuilt binaries download on first compile |

**macOS (current target)**

| Tool | Why |
|------|-----|
| Xcode or Xcode Command Line Tools | `xcodebuild`, Swift toolchain, SDKs |
| [XcodeGen](https://github.com/yonaskolb/XcodeGen) | Generates `Browser.xcodeproj` from `apps/macos/project.yml` |

Install examples (Homebrew on macOS):

```sh
brew install just cmake ninja xcodegen
# Rust: https://rustup.rs/
# Xcode CLT: xcode-select --install
```

Python 3 ships with macOS; on Linux use your distro's `python3` package.

### First run

```sh
git clone https://github.com/korbindeman/ochre.git
cd ochre
just run
```

`just run` builds the SwiftUI app, assembles the CEF bundle into `{name}.app` (`product.toml`, currently `Ochre.app`), and launches it. **CEF must run from an app bundle.** Do not use `cargo run`.

The first build downloads the CEF binary distribution and can take several minutes.

### Common commands

| Command | What it does |
|---------|--------------|
| `just build` | Build the Rust workspace |
| `just test` | Run workspace tests |
| `just verify` | Build, test, lint, and format check |
| `just run` | Build and launch the macOS app (primary dev loop) |
| `just run-native-shell` | Engine-only bisection, embedded CEF without SwiftUI |
| `just dogfood` | Release build → install to `/Applications` → launch |
| `just crash-latest --print` | Print the latest local crash payload |

Run `just --list` for self-tests, formatting, and everything else.

## Contributing

The project is in early development and not yet set up for external contributions. When that opens up, I'll track work in GitHub issues. In the meantime, if you want to experiment: satisfy the [prerequisites](#prerequisites), point an agent at [`CLAUDE.md`](CLAUDE.md), and drive a session. Design rationale lives in [`docs/`](docs/README.md).

## Acknowledgements

- **[Arc](https://arc.net/)**: the interaction model behind this browser.
- **[Glide](https://github.com/glide-browser/glide)**: inspiration for Commands; see [Introducing Glide](https://blog.craigie.dev/introducing-glide/).
- **[I Still Don't Care About Cookies](https://github.com/OhMyGuus/I-Still-Dont-Care-About-Cookies)**: inspiration for cookie-banner handling. This repo doesn't use its code (licensing differs); Ochre's consent feature is a native reimplementation.

## License

[BSD 3-Clause](LICENSE)
