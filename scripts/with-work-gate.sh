#!/usr/bin/env bash
# Run a command only after every work lease has cleared.
#
# Also the single pre-build hook where the codec-enabled CEF is resolved
# (ADR-0040/0042): every build recipe funnels through here, so this is where we
# point cef-dll-sys at the custom build. If the private ochre_cef artifact is
# reachable (scripts/fetch-cef.sh prints its path), CEF_PATH is exported and the
# build embeds proprietary codecs (H.264/AAC). If not — a contributor without
# repo access — nothing is set and cef-dll-sys downloads the stock codec-less
# prebuilt (ADR-0042's retreat). Set CEF_PATH yourself to override; set
# OCHRE_NO_CODECS=1 to force the stock build.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export BROWSER_ROOT="$ROOT"

if [ -z "${CEF_PATH:-}" ] && [ -z "${OCHRE_NO_CODECS:-}" ]; then
  cef="$("$ROOT/scripts/fetch-cef.sh" 2>/dev/null || true)"
  [ -n "$cef" ] && export CEF_PATH="$cef"
fi

exec python3 "$ROOT/scripts/work-lease.py" run "$@"
