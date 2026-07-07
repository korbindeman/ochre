#!/bin/bash
# Post-build phase for the macOS Browser.app (run by Xcode via apps/macos/project.yml).
#
# Xcode has just produced Browser.app with the Swift code linked against
# libbrowser_ffi.dylib. CEF loads its 215 MB framework (dlopen) and launches its
# helper subprocess (exec) from Contents/Frameworks at runtime — neither is
# linked — so we inject them here, embed the dylib, then ad-hoc sign inside-out
# so the (otherwise unsigned) dev build launches.
set -euo pipefail

: "${SRCROOT:?must be run from an Xcode build}"
: "${BUILT_PRODUCTS_DIR:?must be run from an Xcode build}"
: "${FULL_PRODUCT_NAME:?must be run from an Xcode build}"

ROOT="$SRCROOT/../.."
APP="$BUILT_PRODUCTS_DIR/$FULL_PRODUCT_NAME"

if [ "${CONFIGURATION:-Debug}" = "Release" ]; then
  CARGO_TARGET=release
  BUNDLE_DIR=target/bundle-release
else
  CARGO_TARGET=debug
  BUNDLE_DIR=target/bundle
fi

SRC="$ROOT/${BUNDLE_DIR}/browser.app/Contents/Frameworks"
FRAMEWORKS="$APP/Contents/Frameworks"

mkdir -p "$FRAMEWORKS"

echo "[bundle] injecting CEF framework (reshaped as a versioned framework)"
CEF_FW="$FRAMEWORKS/Chromium Embedded Framework.framework"
CEF_BIN="Chromium Embedded Framework"
rm -rf "$CEF_FW"
ditto "$SRC/Chromium Embedded Framework.framework" "$CEF_FW"
# CEF ships a *flat* framework (binary + Resources/ + Libraries/ at the root).
# macOS product validation requires a *versioned* framework
# (Versions/Current/Resources/Info.plist), so reshape it into Versions/A with the
# canonical Current + top-level symlinks. The binary carries its own embedded
# ad-hoc signature, so moving it preserves the signature; CEF is dlopen'd through
# the top-level `Chromium Embedded Framework` symlink at runtime.
mkdir -p "$CEF_FW/Versions/A"
mv "$CEF_FW/$CEF_BIN" "$CEF_FW/Versions/A/$CEF_BIN"
mv "$CEF_FW/Resources" "$CEF_FW/Versions/A/Resources"
mv "$CEF_FW/Libraries" "$CEF_FW/Versions/A/Libraries"
ln -s A "$CEF_FW/Versions/Current"
ln -s "Versions/Current/$CEF_BIN" "$CEF_FW/$CEF_BIN"
ln -s "Versions/Current/Resources" "$CEF_FW/Resources"
ln -s "Versions/Current/Libraries" "$CEF_FW/Libraries"

# CEF derives per-role helper bundle names from the subprocess path — renderers
# launch from "browser Helper (Renderer).app", GPU from "... (GPU).app", etc. —
# so every variant must be present. (With only the generic helper, the renderer
# silently fails to spawn and pages stay blank.)
echo "[bundle] injecting helper subprocess apps (all roles)"
for HELPER in "$SRC"/browser\ Helper*.app; do
  NAME="$(basename "$HELPER")"
  rm -rf "$FRAMEWORKS/$NAME"
  ditto "$HELPER" "$FRAMEWORKS/$NAME"
done

echo "[bundle] embedding libbrowser_ffi.dylib (${CARGO_TARGET})"
ditto "$ROOT/target/${CARGO_TARGET}/libbrowser_ffi.dylib" "$FRAMEWORKS/libbrowser_ffi.dylib"

# Re-sign only the dylib: install_name_tool (pre-build phase) invalidated its
# ad-hoc signature, and dyld refuses to load an arm64 dylib whose signature is
# broken. The CEF framework, helper app, and main executable keep their existing
# valid signatures (CEF's own / cargo's / the linker's). We deliberately do NOT
# seal the app bundle here — Xcode's later CopySwiftLibs phase would invalidate
# any such seal, and `just run-macos` execs the binary directly, which does not
# require a sealed bundle.
echo "[bundle] re-signing libbrowser_ffi.dylib"
codesign --force --sign - "$FRAMEWORKS/libbrowser_ffi.dylib"

# Seal the helper app bundles. cargo's linker ad-hoc-signs each helper Mach-O
# with the bundle flag set but writes no CodeResources, so `codesign -v` fails
# ("no resources but signature indicates they must be present"). Re-signing the
# bundles generates valid seals. (Safe here — CopySwiftLibs only scans the main
# app's Frameworks, never the helpers, so it can't invalidate these afterward.)
echo "[bundle] sealing helper app bundles"
for HELPER in "$FRAMEWORKS"/browser\ Helper*.app; do
  codesign --force --sign - "$HELPER"
done

echo "[bundle] done: $APP"
