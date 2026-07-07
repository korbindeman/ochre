#!/bin/bash
# Install a built Browser.app to /Applications for daily dogfooding.
#
# Quits a running instance first (dev `just run` or a prior dogfood install),
# replaces /Applications/Browser.app via ditto, then launches from there.
set -euo pipefail

SRC="${1:?usage: install-dogfood.sh <path/to/Browser.app>}"
DEST="/Applications/Browser.app"

if [ ! -d "$SRC" ]; then
  echo "[dogfood] error: not found: $SRC" >&2
  exit 1
fi

if pgrep -xq Browser; then
  echo "[dogfood] quitting running Browser..."
  osascript -e 'tell application "Browser" to quit' 2>/dev/null || true
  for _ in $(seq 1 20); do
    pgrep -xq Browser || break
    sleep 0.25
  done
  if pgrep -xq Browser; then
    pkill -x Browser || true
    sleep 0.5
  fi
fi

echo "[dogfood] installing to $DEST"
rm -rf "$DEST"
ditto "$SRC" "$DEST"

echo "[dogfood] launching"
open "$DEST"
