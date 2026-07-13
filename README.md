# Ochre

A minimal, fast, open-source browser with Arc's UX and real native OS apps.

Chromium via CEF. A shared Rust core. Thin native UI per platform: SwiftUI on macOS, Windows Reactor (native WinUI 3 in Rust) on Windows.

**Early development.** macOS MVP in progress. Windows is next.

## Viability

Building a browser is hard — especially for one person. I'm not sure this project is viable in the long run.

So far, though, I'm impressed with what CEF makes possible. Embedding Chromium in a native shell, driving tabs and navigation from a Rust core, shipping real features without maintaining a rendering engine — it works better than I expected. There may be a ceiling eventually: licensing walls, engine limitations, things CEF simply won't let you do. I plan to keep going until I hit it.

I'm also interested in forking an engine outright someday — maybe something like [Ladybird](https://ladybird.org/) if it matures into a realistic base. That's the point where solo development stops being feasible. For now, CEF is the pragmatic path.

## Features

> Snapshot only — may be out of date with the repo.

Ochre is built to be **scriptable and extensible**: a Rust core with a thin native shell, a consent-gated automation surface, and room for users to add their own workflows — including AI — without waiting on us to ship every idea.

- **Arc-style UX** — vertical tabs, spaces, pinned tabs & folders
- **Native adblock & cookie consent** — request-path blocking; reject or hide consent banners
- **Bitwarden / Vaultwarden vault** — unlock, sync, and autofill against your existing server
- **Arc import** — migrate spaces, folders, and tabs from Arc's sidebar export
- **Commands** *(planned)* — user-scriptable automation in the command bar; TypeScript commands with access to tabs, spaces, navigation, clipboard, and more
- **AI** *(experimental)* — exploring what actually helps; won't ship filler. The scriptable surface is the point — users can wire in their own models and tools

**Not yet:**

- **Chrome extensions** — blocked by the engine: CEF removed its extension loader, and the replacement only works with Chrome's own toolbar UI — incompatible with a native shell. Real extension support seems to mean forking all of Chromium. Commands and native features are the interim; extensions stay on the radar.
- **DRM / protected streaming** — blocked by licensing: bundling Widevine is easy, but Netflix and similar services need a Google Widevine agreement and VMP signing that open-source indie browsers have historically been denied. Will look into this eventually.

macOS only for now. Windows is next. The bar for release is dogfooding — it has to hold up as a real daily browser first.

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

The native app owns the window. CEF renders into embedded child views. Windows UI uses Windows Reactor (native WinUI 3 in Rust) on WinUI 3.

| Path | Contents |
|------|----------|
| `crates/core/` | Browser logic, CEF wrapper, tab/space model |
| `crates/ffi/` | C ABI bridge to native frontends |
| `apps/macos/` | SwiftUI macOS app |
| `docs/` | Architecture, roadmap, style guide, ADRs |

See [`docs/architecture.md`](docs/architecture.md) for the full design.

## Development

Development is mostly agentic. You describe a goal in natural language, a feature, a bug, a polish pass, and a coding agent implements it as an end-to-end vertical slice: Rust core, FFI, native UI, menu/shortcut, and verification. Human review and dogfooding sit on top of that loop.

The workflow:

1. **Steer**: name what you want built or fixed. Agents read the design set in `docs/` (architecture, style guide, ADRs) to stay aligned with existing decisions.
2. **Implement**: the agent works the full stack in one pass rather than leaving half-wired stubs. The native UI layer is SwiftUI/AppKit on macOS; browser logic stays in the Rust core.
3. **Verify**: `just verify` for the Rust workspace; `just run` to launch the app; targeted self-tests (`just focus-selftest`, `just redirect-selftest`, etc.) guard regressions called out in the docs.
4. **Dogfood**: `just dogfood` installs a release build to `/Applications` when something is ready for real daily use.

[`CLAUDE.md`](CLAUDE.md) is the operating manual for agents: build commands, architecture constraints, CEF gotchas, and conventions. Point your agent at it before starting a session.

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

`just run` builds the SwiftUI app, assembles the CEF bundle into `Browser.app`, and launches it. **CEF must run from an app bundle.** Do not use `cargo run` directly.

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

The project is in early development and not yet set up for external contributions. When that opens up, work will be tracked in GitHub issues.

Today, development runs through AI coding agents with manual review and dogfooding for quality. If you want to drive a session yourself:

1. Clone the repo and satisfy the [prerequisites](#prerequisites) above.
2. Open the tree in an agent-capable editor (Cursor, Claude Code, etc.).
3. Point the agent at [`CLAUDE.md`](CLAUDE.md), which encodes how to build, test, and stay within project conventions.
4. Describe a concrete slice ("add X", "fix Y", "make Z feel better") and let the agent run the implement → verify loop.

Design rationale and decisions live in [`docs/`](docs/README.md) if you want to read ahead; agents use those automatically.

## Acknowledgements

- **[Arc](https://arc.net/)** — the interaction model this browser is built around.
- **[Glide](https://github.com/glide-browser/glide)** — inspiration for Commands; see [Introducing Glide](https://blog.craigie.dev/introducing-glide/).
- **[I Still Don't Care About Cookies](https://github.com/OhMyGuus/I-Still-Dont-Care-About-Cookies)** — inspiration for cookie-banner handling. Not used directly in this repo (licensing differs); Ochre's consent feature is a native reimplementation.

## License

[BSD 3-Clause](LICENSE)
