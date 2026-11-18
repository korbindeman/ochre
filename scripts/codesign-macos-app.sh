#!/usr/bin/env bash
# Developer ID inside-out codesign for the macOS .app (ADR-0061, rm §6.1).
#
# Called after xcodebuild so CopySwiftLibs has already finished. Does nothing
# (exit 0) when no Developer ID identity is available, so CI/ad-hoc dogfood
# stays unsigned. Never uses `codesign --deep`.
#
# Identity: $BROWSER_CODESIGN_IDENTITY, else the first "Developer ID Application"
# in the keychain. Team ID: $BROWSER_TEAM_ID, else the (TEAMID) suffix on the
# identity name.
#
# Host entitlements (ADR-0070): restricted entitlements
# (com.apple.developer.web-browser*) need an embedded provisioning profile, or
# AMFI SIGKILLs the app at launch. codesign itself never rejects them, so the
# choice is made up front:
#   $BROWSER_PROVISIONING_PROFILE set -> embed it, sign Browser.production.entitlements
#   otherwise                         -> sign Browser.release.entitlements (no restricted keys)
# A final check refuses a signature that carries restricted keys without a profile.
set -euo pipefail

APP="${1:?usage: codesign-macos-app.sh /path/to/Ochre.app}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MACOS="$ROOT/apps/macos"

if [[ ! -d "$APP" ]]; then
  echo "[codesign] error: not an app bundle: $APP" >&2
  exit 1
fi

resolve_identity() {
  if [[ -n "${BROWSER_CODESIGN_IDENTITY:-}" ]]; then
    printf '%s\n' "$BROWSER_CODESIGN_IDENTITY"
    return
  fi
  security find-identity -v -p codesigning 2>/dev/null \
    | awk -F'"' '/Developer ID Application/ { print $2; exit }'
}

IDENTITY="$(resolve_identity || true)"
if [[ -z "$IDENTITY" ]]; then
  echo "[codesign] no Developer ID identity; leaving ad-hoc signature in place"
  echo "[codesign] set BROWSER_CODESIGN_IDENTITY or install a Developer ID Application certificate"
  exit 0
fi

TEAM_ID="${BROWSER_TEAM_ID:-}"
if [[ -z "$TEAM_ID" && "$IDENTITY" =~ \(([A-Z0-9]+)\)$ ]]; then
  TEAM_ID="${BASH_REMATCH[1]}"
fi
if [[ -z "$TEAM_ID" ]]; then
  echo "[codesign] error: could not parse team ID from '$IDENTITY'" >&2
  echo "[codesign] set BROWSER_TEAM_ID to your 10-character Apple team id" >&2
  exit 1
fi

echo "[codesign] identity: $IDENTITY"
echo "[codesign] team id:  $TEAM_ID"

subst_ents() {
  local src="$1"
  local dest="$2"
  # Xcode's AppIdentifierPrefix is "TEAMID." (dot included).
  sed "s/\$(AppIdentifierPrefix)/${TEAM_ID}./g" "$src" > "$dest"
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
subst_ents "$MACOS/Browser.production.entitlements" "$TMP/production.entitlements"
subst_ents "$MACOS/Browser.release.entitlements" "$TMP/release.entitlements"

sign() {
  local path="$1"
  shift
  local err="$TMP/last-sign.err"
  if codesign --force --options runtime --timestamp --sign "$IDENTITY" "$@" "$path" 2>"$err"; then
    return 0
  fi
  if grep -qi timestamp "$err"; then
    echo "[codesign] timestamp server unreachable; retrying without a timestamp"
    codesign --force --options runtime --sign "$IDENTITY" "$@" "$path"
    return
  fi
  cat "$err" >&2
  return 1
}

FRAMEWORKS="$APP/Contents/Frameworks"

# The engine is the system's WebKit (ADR-0072): the only nested code is the
# Rust core dylib, signed first, then the host.
echo "[codesign] libbrowser_ffi.dylib"
sign "$FRAMEWORKS/libbrowser_ffi.dylib"

echo "[codesign] host app"
PROFILE="${BROWSER_PROVISIONING_PROFILE:-}"
if [[ -n "$PROFILE" ]]; then
  if [[ ! -f "$PROFILE" ]]; then
    echo "[codesign] error: BROWSER_PROVISIONING_PROFILE not found: $PROFILE" >&2
    exit 1
  fi
  cp "$PROFILE" "$APP/Contents/embedded.provisionprofile"
  echo "[codesign] embedded provisioning profile; signing restricted entitlements"
  sign "$APP" --entitlements "$TMP/production.entitlements"
else
  rm -f "$APP/Contents/embedded.provisionprofile"
  echo "[codesign] no provisioning profile; signing without restricted entitlements (passkeys stay off)"
  sign "$APP" --entitlements "$TMP/release.entitlements"
fi

# Guard (ADR-0070): restricted keys without a profile launch-kill the app.
ents="$(codesign -d --entitlements :- "$APP" 2>/dev/null || true)"
if grep -Eq "com\.apple\.developer\." <<<"$ents" \
  && [[ ! -f "$APP/Contents/embedded.provisionprofile" ]]; then
  echo "[codesign] error: restricted entitlements signed without a provisioning profile; AMFI would kill the app at launch" >&2
  exit 1
fi

codesign --verify --strict --verbose=2 "$APP"
echo "[codesign] done: $APP"
