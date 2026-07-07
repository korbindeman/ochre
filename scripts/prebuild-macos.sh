#!/bin/bash
# Pre-build phase for the macOS Browser.app (run by Xcode via apps/macos/project.yml).
#
# Executes before Xcode compiles/links the Swift target, so it must leave the
# Rust artifacts in place that the link and post-build steps depend on:
#
#   1. libbrowser_ffi.dylib — the C-ABI dylib the app links against (-lbrowser_ffi).
#   2. Its install name rewritten to @rpath/... — cargo bakes an absolute
#      target/{debug,release}/deps path into LC_ID_DYLIB, which the linker would
#      copy into the executable verbatim; @rpath lets it resolve from
#      Contents/Frameworks.
#   3. target/bundle[/ -release]/browser.app — the authoritative source (via
#      `bin/bundle.rs`) for the CEF framework + helper apps the post-build step
#      injects. Rebuilt every time so the helper binary never goes stale.
#
# Xcode sets CONFIGURATION (Debug / Release); `just dogfood` builds Release.
set -euo pipefail

# Xcode script phases run with a minimal PATH; cargo lives under ~/.cargo/bin.
export PATH="$HOME/.cargo/bin:$PATH"

# Resolve the workspace root. Under Xcode, SRCROOT is apps/macos; standalone,
# this script lives in <root>/scripts.
if [ -n "${SRCROOT:-}" ]; then
  cd "$SRCROOT/../.."
else
  cd "$(cd "$(dirname "$0")/.." && pwd)"
fi

if [ "${CONFIGURATION:-Debug}" = "Release" ]; then
  CARGO_PROFILE=(--release)
  CARGO_TARGET=release
  BUNDLE_FLAG=(--release)
  BUNDLE_DIR=target/bundle-release
else
  CARGO_PROFILE=()
  CARGO_TARGET=debug
  BUNDLE_FLAG=()
  BUNDLE_DIR=target/bundle
fi

echo "[prebuild] building libbrowser_ffi.dylib (${CARGO_TARGET})"
if ((${#CARGO_PROFILE[@]})); then
  cargo build -p browser-ffi "${CARGO_PROFILE[@]}"
else
  cargo build -p browser-ffi
fi

echo "[prebuild] pinning dylib install name to @rpath/libbrowser_ffi.dylib"
install_name_tool -id @rpath/libbrowser_ffi.dylib "target/${CARGO_TARGET}/libbrowser_ffi.dylib"

echo "[prebuild] assembling ${BUNDLE_DIR}/browser.app (CEF framework + helper)"
if ((${#BUNDLE_FLAG[@]})); then
  cargo run --quiet --bin bundle -- "${BUNDLE_FLAG[@]}"
else
  cargo run --quiet --bin bundle
fi
