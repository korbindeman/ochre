#!/usr/bin/env bash
# Notarize and staple a Developer ID signed .app (ADR-0070), so it opens on
# another Mac without a Gatekeeper warning or `xattr` step.
#
# Usage: scripts/notarize-macos-app.sh /path/to/Ochre.app
#
# Credentials, first match wins:
#   NOTARY_KEY_ID + NOTARY_ISSUER_ID + NOTARY_KEY (App Store Connect API key,
#     the .p8 file contents)                      - what CI uses
#   NOTARY_PROFILE (a `xcrun notarytool store-credentials` keychain profile)
# With none set it exits 0 and says so, unless NOTARY_REQUIRED=1.
set -euo pipefail

APP="${1:?usage: notarize-macos-app.sh /path/to/App.app}"
[[ -d "$APP" ]] || { echo "[notarize] error: not an app bundle: $APP" >&2; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

auth=()
if [[ -n "${NOTARY_KEY_ID:-}" && -n "${NOTARY_ISSUER_ID:-}" && -n "${NOTARY_KEY:-}" ]]; then
  printf '%s\n' "$NOTARY_KEY" >"$TMP/key.p8"
  chmod 600 "$TMP/key.p8"
  auth=(--key "$TMP/key.p8" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID")
elif [[ -n "${NOTARY_PROFILE:-}" ]]; then
  auth=(--keychain-profile "$NOTARY_PROFILE")
else
  if [[ "${NOTARY_REQUIRED:-0}" == "1" ]]; then
    echo "[notarize] error: no notarization credentials (NOTARY_KEY_ID/NOTARY_ISSUER_ID/NOTARY_KEY or NOTARY_PROFILE)" >&2
    exit 1
  fi
  echo "[notarize] no credentials; skipping (the app will need xattr -cr on other Macs)"
  exit 0
fi

# Notarization rejects anything not Developer ID signed with hardened runtime.
if ! codesign -dv "$APP" 2>&1 | grep -q "Authority=Developer ID Application"; then
  echo "[notarize] error: $APP is not Developer ID signed; run scripts/codesign-macos-app.sh first" >&2
  exit 1
fi

zip="$TMP/$(basename "$APP" .app).zip"
ditto -c -k --keepParent "$APP" "$zip"
echo "[notarize] submitting $(du -h "$zip" | awk '{print $1}') to Apple (this takes a few minutes)"
if ! xcrun notarytool submit "$zip" "${auth[@]}" --wait --output-format json >"$TMP/result.json"; then
  cat "$TMP/result.json" >&2 || true
fi
status="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("status",""))' "$TMP/result.json" 2>/dev/null || true)"
if [[ "$status" != "Accepted" ]]; then
  id="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("id",""))' "$TMP/result.json" 2>/dev/null || true)"
  echo "[notarize] error: notarization status '${status:-unknown}'" >&2
  [[ -n "$id" ]] && xcrun notarytool log "$id" "${auth[@]}" >&2 || true
  exit 1
fi

xcrun stapler staple "$APP"
xcrun stapler validate "$APP"
# What a friend's Mac will check on first open.
spctl --assess --type execute --verbose=2 "$APP"
echo "[notarize] done: $APP"
