#!/bin/bash
# Install a built .app to /Applications for daily dogfooding.
#
# Quits a running instance first (dev `just run` or a prior dogfood install),
# replaces /Applications/<Name>.app via ditto, then launches from there.
# The installed name follows the .app basename (product.toml → PRODUCT_NAME).
set -euo pipefail

SRC="${1:?usage: install-dogfood.sh <path/to/App.app>}"

if [ ! -d "$SRC" ]; then
  echo "[dogfood] error: not found: $SRC" >&2
  exit 1
fi

APP_NAME="$(basename "$SRC" .app)"
DEST="/Applications/${APP_NAME}.app"

if pgrep -xq "$APP_NAME"; then
  echo "[dogfood] quitting running ${APP_NAME}..."
  osascript -e "tell application \"${APP_NAME}\" to quit" 2>/dev/null || true
  for _ in $(seq 1 20); do
    pgrep -xq "$APP_NAME" || break
    sleep 0.25
  done
  if pgrep -xq "$APP_NAME"; then
    pkill -x "$APP_NAME" || true
    sleep 0.5
  fi
fi

echo "[dogfood] installing to $DEST"
rm -rf "$DEST"
ditto "$SRC" "$DEST"

echo "[dogfood] launching"
open "$DEST"
