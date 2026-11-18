# Justfile - build orchestration for the browser monorepo.
# Architecture spec §18: per-platform headless build paths, no IDE required.
#
# Prerequisites: cargo, just. macOS builds also need XcodeGen + xcodebuild (CLT).

# Default: show available recipes.
default:
    @just --list

# Product display name (`.app` / executable from product.toml). Xcode scheme/target stays `Browser` (ADR-0060).
app_name := `python3 scripts/product.py name`

# ── Parallel-agent work leases ──────────────────────────────────────
# Claim at slice start; release before verify/build. Recipes below wait until
# every lease is gone. Cargo's own lock handles simultaneous compiles.

# Claim a work lease for this session (optional task label for status).
work-begin *TASK:
    ./scripts/work-lease.py begin {{TASK}}

# Refresh this session's lease heartbeat (long slices).
work-touch:
    ./scripts/work-lease.py touch

# Release this session's lease - required before just verify / just build.
work-end:
    ./scripts/work-lease.py end

# List sessions still editing.
work-status:
    ./scripts/work-lease.py status

# ── Rust workspace ──────────────────────────────────────────────────

# Build all Rust crates.
build:
    ./scripts/with-work-gate.sh cargo build --workspace

# Run all tests.
test:
    ./scripts/with-work-gate.sh cargo test --workspace

# Lint with clippy.
lint:
    ./scripts/with-work-gate.sh cargo clippy --workspace -- -D warnings

# Format check.
fmt-check:
    cargo fmt --all -- --check

# Format write.
fmt:
    cargo fmt --all

# ── Run ─────────────────────────────────────────────────────────────

# On macOS this builds the SwiftUI app (WebKit via the Rust core, ADR-0072) - the
# actual product - and execs the binary directly (not via `open`) so the core's
# stdout/stderr stay in the terminal.
#
# Run the native development app for the current platform.
run: gen-macos
    ./scripts/with-work-gate.sh bash -c 'cd apps/macos && xcodebuild -scheme Browser -configuration Debug -derivedDataPath build.noindex build'
    ./apps/macos/build.noindex/Build/Products/Debug/{{app_name}}.app/Contents/MacOS/{{app_name}}

# ── macOS app (SwiftUI) ─────────────────────────────────────────────
# Requires XcodeGen + Xcode Command Line Tools.

# Generate the Xcode project from project.yml (XcodeGen).
gen-macos:
    cd apps/macos && xcodegen generate

# Build the macOS app without running it (generate project first if missing).
build-macos: gen-macos
    ./scripts/with-work-gate.sh bash -c 'cd apps/macos && xcodebuild -scheme Browser -derivedDataPath build.noindex build'

# P0-1 focus-routing regression (ADR-0018): builds the app and runs the in-process
# synthetic-input focus self-test, which drives page ↔ command-bar ↔ tab-switch ↔
# ⌘L transitions and asserts the page reclaims keyboard focus each time. Exits
# non-zero on any failure. Runs under a throwaway $HOME so it never touches your
# real profile.
focus-selftest: gen-macos
    ./scripts/with-work-gate.sh bash -c 'cd apps/macos && xcodebuild -scheme Browser -configuration Debug -derivedDataPath build.noindex build'
    HOME="$(mktemp -d)" ./apps/macos/build.noindex/Build/Products/Debug/{{app_name}}.app/Contents/MacOS/{{app_name}} --focus-selftest

# P1-27 sidebar drag regression (ADR-0035): builds the app and runs the in-process
# drag self-test, which drives reorder / save / folder / favorite / cancel drags
# through the gesture pipeline against live row frames and asserts the core's
# resulting structure after each. Exits non-zero on any failure. Throwaway $HOME
# so it never touches your real profile.
drag-selftest: gen-macos
    ./scripts/with-work-gate.sh bash -c 'cd apps/macos && xcodebuild -scheme Browser -configuration Debug -derivedDataPath build.noindex build'
    HOME="$(mktemp -d)" ./apps/macos/build.noindex/Build/Products/Debug/{{app_name}}.app/Contents/MacOS/{{app_name}} --drag-selftest

# P1-30 ⌘W close-path regression (ADR-0037): builds the app and runs the
# in-process close self-test, which drives ⌘W through the menu against an
# ordinary tab, an active saved tab (the suspend path), and the empty canvas, asserting the window only
# closes from the empty canvas. Exits non-zero on any failure. Throwaway $HOME
# so it never touches your real profile.
close-selftest: gen-macos
    ./scripts/with-work-gate.sh bash -c 'cd apps/macos && xcodebuild -scheme Browser -configuration Debug -derivedDataPath build.noindex build'
    HOME="$(mktemp -d)" ./apps/macos/build.noindex/Build/Products/Debug/{{app_name}}.app/Contents/MacOS/{{app_name}} --close-selftest

# New-tab and overlay commit paths: direct open, command-bar commit, and the
# ADR-0068 late-close regression (an address edit must survive the command
# bar's late close and navigate the current tab). Loads example.* pages.
newtab-selftest: gen-macos
    ./scripts/with-work-gate.sh bash -c 'cd apps/macos && xcodebuild -scheme Browser -configuration Debug -derivedDataPath build.noindex build'
    HOME="$(mktemp -d)" ./apps/macos/build.noindex/Build/Products/Debug/{{app_name}}.app/Contents/MacOS/{{app_name}} --newtab-selftest

# Little-window regression (P1-45, ADR-0067): opens a little window, checks
# it stays out of the sidebar, promotes it with Cmd+O semantics (live page kept),
# and closes another. Throwaway $HOME so it never touches the real session.
little-selftest: gen-macos
    ./scripts/with-work-gate.sh bash -c 'cd apps/macos && xcodebuild -scheme Browser -configuration Debug -derivedDataPath build.noindex build'
    HOME="$(mktemp -d)" ./apps/macos/build.noindex/Build/Products/Debug/{{app_name}}.app/Contents/MacOS/{{app_name}} --little-selftest

# ── Dogfood (release) ───────────────────────────────────────────────

# The artifact `dogfood` installs and the alpha-release CI workflow publishes -
# one definition so local and CI can't drift.
#
# Release build of the macOS .app (no install).
build-release: gen-macos
    ./scripts/with-work-gate.sh bash -c 'cd apps/macos && xcodebuild -scheme Browser -configuration Release -derivedDataPath build.noindex build'
    ./scripts/codesign-macos-app.sh apps/macos/build.noindex/Build/Products/Release/{{app_name}}.app

# Release build, install to /Applications, and launch.
# Replaces any existing /Applications/{{app_name}}.app (quits a running instance first).
dogfood: build-release
    ./scripts/install-dogfood.sh apps/macos/build.noindex/Build/Products/Release/{{app_name}}.app

# ── Convenience ─────────────────────────────────────────────────────

# Path to the latest local crash payload JSON (debug builds use "{name} Dev").
# Pass --print to dump the file contents.
crash-latest *ARGS:
    ./scripts/latest-crash.sh {{ARGS}}

# Apply product.toml to generated identity files (ADR-0060).
sync-product:
    python3 scripts/product.py sync

# Assert generated identity files and live product-name surfaces match product.toml.
product-check:
    python3 scripts/product.py check

# No em-dash rule (CLAUDE.md): fail if any tracked file contains U+2014.
# The pattern is built via printf so this justfile doesn't contain one itself.
emdash-check:
    @if git grep -In "$(printf '\342\200\224')" -- .; then \
        echo "error: em-dash (U+2014) found in the files above; this repo bans it (see CLAUDE.md)"; \
        exit 1; \
    fi

# Privacy usage strings must sit in the main app's Info.plist: TCC attributes a web
# content process's camera/mic/Bluetooth/location request to the app and aborts without them.
privacy-check:
    @for k in NSMicrophoneUsageDescription NSCameraUsageDescription NSBluetoothAlwaysUsageDescription NSLocationUsageDescription NSLocationWhenInUseUsageDescription; do \
        grep -q "^        $k: " apps/macos/project.yml || { echo "error: $k missing from apps/macos/project.yml info.properties (TCC aborts the app without it)"; exit 1; }; \
    done

# Verify: build + test + lint + style checks, the full CI-style check.
verify: build test lint fmt-check emdash-check product-check privacy-check
    @echo "✓ Workspace verified: build, test, lint, fmt-check, emdash-check, product-check, privacy-check all passed."
