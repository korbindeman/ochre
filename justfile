# Justfile — build orchestration for the browser monorepo.
# Architecture spec §18: per-platform headless build paths, no IDE required.
#
# Prerequisites: cargo, just. macOS builds also need XcodeGen + xcodebuild (CLT).

# Default: show available recipes.
default:
    @just --list

# ── Rust workspace ──────────────────────────────────────────────────

# Build all Rust crates.
build:
    cargo build --workspace

# Run all tests.
test:
    cargo test --workspace

# Lint with clippy.
lint:
    cargo clippy --workspace -- -D warnings

# Format check.
fmt-check:
    cargo fmt --all -- --check

# Format write.
fmt:
    cargo fmt --all

# ── Run ─────────────────────────────────────────────────────────────

# On macOS this builds the SwiftUI Browser.app (embedded CEF via FFI) — the actual
# product — and execs the binary directly (not via `open`) so CEF's + the core's
# stdout/stderr stay in the terminal.
#
# Run the native development app for the current platform.
run: gen-macos
    cd apps/macos && xcodebuild -scheme Browser -configuration Debug -derivedDataPath build build
    ./apps/macos/build/Build/Products/Debug/Browser.app/Contents/MacOS/Browser

# A pure-Rust AppKit NSWindow hosting the embedded browser (native_shell.rs) — the
# same embed.rs hosting path the SwiftUI app uses, minus SwiftUI + FFI. Reach for
# it to answer "is a blank page / CEF issue in the engine or the Swift layer?" CEF
# must run from a .app bundle, so this assembles target/bundle/browser.app first.
#
# Engine-isolation bisection tool (embedded CEF, no SwiftUI).
run-native-shell: bundle
    ./target/bundle/browser.app/Contents/MacOS/browser --native-shell

# Assemble target/bundle/browser.app (builds browser + browser_helper).
bundle:
    cargo run --bin bundle

# ── macOS app (SwiftUI) ─────────────────────────────────────────────
# Requires XcodeGen + Xcode Command Line Tools.

# Generate the Xcode project from project.yml (XcodeGen).
gen-macos:
    cd apps/macos && xcodegen generate

# Build the macOS app without running it (generate project first if missing).
build-macos: gen-macos
    cd apps/macos && xcodebuild -scheme Browser build

# ── Convenience ─────────────────────────────────────────────────────

# Verify: build + test + lint — the full CI-style check.
verify: build test lint fmt-check
    @echo "✓ Workspace verified: build, test, lint, fmt-check all passed."
