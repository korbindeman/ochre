#!/usr/bin/env bash
# Build CEF WITH proprietary codecs (H.264 / AAC) + PGO for macOS arm64, from source.
#
# Why this exists: the stock prebuilt CEF (what cef-dll-sys downloads) ships
# proprietary codecs disabled, so most MP4/H.264 web video doesn't play. This
# produces a codec-enabled, PGO-optimized minimal binary distribution that the
# crate consumes via CEF_PATH. See docs/adr/0040 (build) and 0042 (distribution).
#
# Target coordinates must match the cef/cef-dll-sys crate pin exactly, or the
# artifact won't load. The crate version's build metadata (+148.0.10 in
# Cargo.lock) IS the CEF version. Look the branch/commit up in
# https://cef-builds.spotifycdn.com/index.json (macosarm64 entry whose
# cef_version starts with the crate's CEF version):
#
#   CEF 148.0.10  commit 7ee53f5  Chromium 148.0.7778.218  ->  CEF branch 7778
#
# Re-run per CEF pin bump (ADR-0028 gates bumps): update CEF_BRANCH/CEF_CHECKOUT
# from the index, re-run, re-gate with the selftests, re-upload.
#
# Runs unattended for ~5-10 h. Launch detached, e.g.:
#   nohup caffeinate -i -s ./build-cef-codecs.sh > ~/cef-build/build.log 2>&1 &
#
# --- Two non-obvious prerequisites this script handles (both learned the hard way) ---
# 1. Metal Toolchain: since Xcode 26 the `metal` compiler is a separate on-demand
#    component (not bundled). ANGLE's Metal shaders won't build without it. Installs
#    user-level, no sudo. Symptom if missing: "cannot execute tool 'metal'".
# 2. PGO profiles: is_official_build=true enables PGO, which needs profile data that
#    automate-git's generated .gclient does NOT fetch (checkout_pgo_profiles=False).
#    So we build in two phases: (1) checkout with PGO off, (2) enable the profile
#    var, fetch it via runhooks, then build with PGO on. Symptom if skipped:
#    gn gen fails at build/config/compiler/pgo/BUILD.gn.
set -euo pipefail

CEF_BRANCH="${CEF_BRANCH:-7778}"
CEF_CHECKOUT="${CEF_CHECKOUT:-7ee53f5}"
ROOT="${ROOT:-$HOME/cef-build}"
DEPOT="$ROOT/depot_tools"
DL="$ROOT/chromium_git"

export PATH="$DEPOT:$PATH"
export CEF_USE_GN=1
export CEF_ARCHIVE_FORMAT=tar.bz2

# proprietary_codecs + ffmpeg_branding=Chrome: the whole point (H.264/AAC).
# use_thin_lto=false keeps the official-build link within a 16 GB host's RAM
# (ThinLTO link is the memory blow-up on constrained boxes; ADR-0040). If the
# link still OOMs, drop to a non-official release build: replace is_official_build
# =true with `is_official_build=false is_debug=false`.
# symbol_level=0 trims RAM/disk/time; a minimal distribution carries no symbols.
GN_BASE="is_official_build=true proprietary_codecs=true ffmpeg_branding=Chrome use_thin_lto=false symbol_level=0"
GN_NOPGO="$GN_BASE chrome_pgo_phase=0"   # phase 1: PGO off (no profile yet)
GN_PGO="$GN_BASE"                         # phase 2: is_official_build defaults PGO on

echo "=== CEF codec+PGO build @ $(date) ==="
echo "branch=$CEF_BRANCH checkout=$CEF_CHECKOUT"

# --- Prereq: Metal Toolchain (Xcode 26+) ---
if ! xcrun -sdk macosx metal --version >/dev/null 2>&1; then
  echo "=== installing Metal Toolchain (user-level, no sudo) ==="
  xcodebuild -downloadComponent MetalToolchain
fi

# --- depot_tools + automate-git.py ---
mkdir -p "$ROOT"; cd "$ROOT"
[ -d "$DEPOT" ] || git clone --depth 1 https://chromium.googlesource.com/chromium/tools/depot_tools.git "$DEPOT"
curl -fsSL -o automate-git.py \
  "https://bitbucket.org/chromiumembedded/cef/raw/${CEF_BRANCH}/tools/automate/automate-git.py"

COMMON=(--download-dir="$DL" --depot-tools-dir="$DEPOT"
        --branch="$CEF_BRANCH" --checkout="$CEF_CHECKOUT"
        --arm64-build --no-debug-build --no-release-tests --no-distrib-docs
        --minimal-distrib-only)

# --- Phase 1: checkout + deps + gn (PGO off, so the gn hook doesn't need a profile) ---
mkdir -p "$DL"
echo "=== phase 1: checkout (PGO off, --no-build) ==="
GN_DEFINES="$GN_NOPGO" python3 automate-git.py "${COMMON[@]}" --no-build

# --- Phase 2: enable + fetch PGO profiles, then build with PGO on ---
echo "=== phase 2: enable PGO profiles + runhooks ==="
sed -i.bak "s/'checkout_pgo_profiles': False/'checkout_pgo_profiles': True/" "$DL/chromium/.gclient"
( cd "$DL/chromium" && GN_DEFINES="$GN_NOPGO" gclient runhooks )

echo "=== phase 2: build + distrib (PGO on) ==="
GN_DEFINES="$GN_PGO" python3 automate-git.py "${COMMON[@]}" --no-update --force-build --force-distrib

echo "=== build finished @ $(date) ==="
echo "distribution(s):"
find "$DL/chromium/src/cef/binary_distrib" -maxdepth 1 -type d -name 'cef_binary_*' 2>/dev/null || true
