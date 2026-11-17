#!/usr/bin/env bash
# Fetch the codec-enabled CEF distribution from the private ochre_cef repo and
# print the absolute CEF_PATH to consume it. See docs/adr/0040 + 0042.
#
#   Local dev:  export CEF_PATH="$(scripts/fetch-cef.sh)"   then  just run / just build
#   CI:         same, with a read-only PAT for ochre_cef in GH_TOKEN.
#
# The CEF version is derived from the cef-dll-sys pin in Cargo.lock (the build
# metadata after '+', e.g. 148.4.0+148.0.10 -> 148.0.10), which is the ochre_cef
# release tag. Override with CEF_VERSION=... if needed.
#
# Fallback (ADR-0042's contributor path): if the repo isn't reachable - gh not
# installed, not authenticated, or no read access - this prints nothing and exits
# 0. With CEF_PATH unset, cef-dll-sys downloads the stock (codec-less) prebuilt,
# so the browser still builds, just without proprietary-codec playback.
set -euo pipefail

REPO="korbindeman/ochre_cef"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE="${OCHRE_CEF_CACHE:-$HOME/.cache/ochre/cef}"

VERSION="${CEF_VERSION:-$(awk '/name = "cef-dll-sys"/{getline; sub(/.*\+/,""); sub(/".*/,""); print; exit}' "$ROOT/Cargo.lock")}"
if [ -z "$VERSION" ]; then
  echo "fetch-cef: could not determine CEF version from Cargo.lock" >&2; exit 0
fi

DEST="$CACHE/$VERSION"

# Already extracted? Print and done (idempotent - cheap to call every build).
# (find exits non-zero when DEST doesn't exist yet; tolerate that under set -e.)
existing="$(find "$DEST" -maxdepth 1 -type d -name 'cef_binary_*_minimal' 2>/dev/null | head -1 || true)"
if [ -n "$existing" ]; then echo "$existing"; exit 0; fi

# Need gh + read access to the private repo; otherwise fall back to stock prebuilt.
if ! command -v gh >/dev/null 2>&1 || ! gh release view "$VERSION" -R "$REPO" >/dev/null 2>&1; then
  echo "fetch-cef: ochre_cef release '$VERSION' not reachable - using stock codec-less CEF" >&2
  exit 0
fi

mkdir -p "$DEST"
gh release download "$VERSION" -R "$REPO" -p '*.tar.gz' -D "$DEST" --clobber >&2
tar -xzf "$DEST"/cef_binary_*_minimal.tar.gz -C "$DEST"
rm -f "$DEST"/cef_binary_*_minimal.tar.gz

result="$(find "$DEST" -maxdepth 1 -type d -name 'cef_binary_*_minimal' 2>/dev/null | head -1 || true)"
if [ -n "$result" ]; then
  # cef-dll-sys expects the framework at the distribution ROOT, not under Release/.
  # Its own downloader flattens Release/ up (download-cef extract_target_archive:
  # `rename(Release, cef_dir)`), and `cef::build_util::mac::bundle` copies the
  # framework from <CEF_DIR>/Chromium Embedded Framework.framework. Replicate that
  # here, or `just bundle` fails with "No such file or directory".
  if [ -d "$result/Release" ]; then mv "$result/Release/"* "$result/" && rmdir "$result/Release"; fi
  if [ -d "$result/Resources" ]; then mv "$result/Resources/"* "$result/" 2>/dev/null || true; rmdir "$result/Resources" 2>/dev/null || true; fi
  echo "$result"
fi
