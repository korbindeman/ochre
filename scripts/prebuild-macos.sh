#!/bin/bash
# Pre-build phase for the macOS Browser.app (run by Xcode via apps/macos/project.yml).
#
# Executes before Xcode compiles/links the Swift target, so it must leave the
# Rust artifacts in place that the link and post-build steps depend on:
#
#   1. libbrowser_ffi.dylib — the C-ABI dylib the app links against (-lbrowser_ffi).
#   2. Its install name rewritten to @rpath/... — cargo bakes an absolute
#      target/debug/deps path into LC_ID_DYLIB, which the linker would copy into
#      the executable verbatim; @rpath lets it resolve from Contents/Frameworks.
#   3. target/bundle/browser.app — the authoritative source (via cef-rs's own
#      bundler) for the CEF framework + `browser Helper.app` the post-build step
#      injects. Rebuilt every time so the helper binary never goes stale.
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

echo "[prebuild] building libbrowser_ffi.dylib"
cargo build -p browser-ffi

echo "[prebuild] pinning dylib install name to @rpath/libbrowser_ffi.dylib"
install_name_tool -id @rpath/libbrowser_ffi.dylib target/debug/libbrowser_ffi.dylib

echo "[prebuild] assembling target/bundle/browser.app (CEF framework + helper)"
cargo run --quiet --bin bundle
