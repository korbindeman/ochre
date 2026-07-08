#!/bin/bash
# Align CEF helper bundle IDs with the host app (Chromium-style suffixes).
#
# The standalone `bundle` binary lays down helpers with the main app's
# CFBundleIdentifier. macOS app-usage tracking expects helpers to live under the
# same identifier family (e.g. com.example.Browser.helper.renderer), not a stale
# dev-only ID that doesn't match the SwiftUI shell.
#
# Usage: patch-helper-bundle-ids.sh /path/to/Browser.app
set -euo pipefail

APP="${1:?app bundle path}"
MAIN_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Contents/Info.plist")"
FRAMEWORKS="$APP/Contents/Frameworks"

helper_suffix() {
  case "$1" in
    "browser Helper") echo ".helper" ;;
    "browser Helper (GPU)") echo ".helper.gpu" ;;
    "browser Helper (Renderer)") echo ".helper.renderer" ;;
    "browser Helper (Plugin)") echo ".helper.plugin" ;;
    "browser Helper (Alerts)") echo ".helper.alerts" ;;
    *) echo ".helper" ;;
  esac
}

shopt -s nullglob
for helper in "$FRAMEWORKS"/browser\ Helper*.app; do
  base="$(basename "$helper" .app)"
  suffix="$(helper_suffix "$base")"
  plist="$helper/Contents/Info.plist"
  echo "[patch-bundle-id] ${base} → ${MAIN_ID}${suffix}"
  /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier ${MAIN_ID}${suffix}" "$plist"
  # Info.plist edits invalidate the bundle seal; re-sign so dyld accepts the helper.
  codesign --force --sign - "$helper"
done
