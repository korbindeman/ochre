#!/usr/bin/env bash
# Run a command only after every work lease has cleared.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export BROWSER_ROOT="$ROOT"
exec python3 "$ROOT/scripts/work-lease.py" run "$@"
