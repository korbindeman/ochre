#!/bin/bash
# Post-build phase for the macOS .app (run by Xcode via apps/macos/project.yml).
#
# Xcode has just produced the host .app with the Swift code linked against
# libbrowser_ffi.dylib. Embed the dylib in Contents/Frameworks and re-sign it
# ad hoc: install_name_tool (pre-build phase) invalidated its signature, and
# dyld refuses an arm64 dylib whose signature is broken. The web engine is the
# system's WebKit (ADR-0072), so nothing else is injected.
set -euo pipefail

: "${SRCROOT:?must be run from an Xcode build}"
: "${BUILT_PRODUCTS_DIR:?must be run from an Xcode build}"
: "${FULL_PRODUCT_NAME:?must be run from an Xcode build}"

ROOT="$SRCROOT/../.."
APP="$BUILT_PRODUCTS_DIR/$FULL_PRODUCT_NAME"

if [ "${CONFIGURATION:-Debug}" = "Release" ]; then
  CARGO_TARGET=release
else
  CARGO_TARGET=debug
fi

FRAMEWORKS="$APP/Contents/Frameworks"
mkdir -p "$FRAMEWORKS"

# Builds made before ADR-0072 carried CEF here; never ship a stale copy.
rm -rf "$FRAMEWORKS/Chromium Embedded Framework.framework" "$FRAMEWORKS"/browser\ Helper*.app

echo "[bundle] embedding libbrowser_ffi.dylib (${CARGO_TARGET})"
ditto "$ROOT/target/${CARGO_TARGET}/libbrowser_ffi.dylib" "$FRAMEWORKS/libbrowser_ffi.dylib"

echo "[bundle] re-signing libbrowser_ffi.dylib"
codesign --force --sign - "$FRAMEWORKS/libbrowser_ffi.dylib"

echo "[bundle] done: $APP"
