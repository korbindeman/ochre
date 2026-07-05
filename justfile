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

# Build in release mode.
build-release:
    cargo build --workspace --release

# Run all tests.
test:
    cargo test --workspace

# Run tests for a specific crate (e.g. `just test-one core`).
test-one crate:
    cargo test -p browser-{{crate}}

# Lint with clippy.
lint:
    cargo clippy --workspace -- -D warnings

# Format check.
fmt-check:
    cargo fmt --all -- --check

# Format write.
fmt:
    cargo fmt --all

# Clean build artifacts.
clean:
    cargo clean

# ── macOS app (SwiftUI) ─────────────────────────────────────────────
# Requires XcodeGen + Xcode Command Line Tools.

# Generate the Xcode project from project.yml (XcodeGen).
gen-macos:
    cd apps/macos && xcodegen generate

# Build the macOS app (generate project first if missing).
build-macos: gen-macos
    cd apps/macos && xcodebuild -scheme Browser build

# ── Convenience ─────────────────────────────────────────────────────

# Verify: build + test + lint — the full CI-style check.
verify: build test lint fmt-check
    @echo "✓ Workspace verified: build, test, lint, fmt-check all passed."
