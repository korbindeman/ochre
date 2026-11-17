#!/usr/bin/env bash
# Build the Tauri-compatible auto-update artifacts for a release (ADR-0052):
#   <App>.app.tar.gz  - signed archive the updater downloads
#   latest.json       - manifest (version, notes, platform url + signature)
#
# Usage:
#   scripts/package-update-artifacts.sh <tag> <path-to-App.app> <out-dir>
#
# Env:
#   UPDATE_SIGNING_PRIVATE_KEY   - minisign secret key file contents (required to sign)
#   UPDATE_SIGNING_PRIVATE_KEY_PASSWORD - optional password (empty for unencrypted)
#   RELEASE_NOTES - optional notes string for latest.json
set -euo pipefail

tag="${1:?tag e.g. v0.1.0-alpha.1}"
app="${2:?path to App.app}"
out="${3:?output directory}"

if [[ ! -d "$app" ]]; then
  echo "error: app bundle not found at $app" >&2
  exit 1
fi

version="${tag#v}"
mkdir -p "$out"
app_parent="$(cd "$(dirname "$app")" && pwd)"
app_name="$(basename "$app")"
archive="$out/${app_name}.tar.gz"

# Tar the .app so the updater can extract a full bundle (same shape as Tauri).
tar -czf "$archive" -C "$app_parent" "$app_name"
echo "Wrote $archive ($(du -h "$archive" | awk '{print $1}'))"

sig_b64=""
if [[ -z "${UPDATE_SIGNING_PRIVATE_KEY:-}" ]]; then
  echo "warning: UPDATE_SIGNING_PRIVATE_KEY unset - latest.json signature will be empty" >&2
else
  key_file="$(mktemp)"
  sig_file="$(mktemp)"
  cleanup() { rm -f "$key_file" "$sig_file" 2>/dev/null || true; }
  trap cleanup EXIT
  # Preserve exact key file bytes (minisign is picky about newlines).
  printf '%s' "$UPDATE_SIGNING_PRIVATE_KEY" >"$key_file"
  # Ensure trailing newline if the secret was pasted without one.
  [[ -s "$key_file" && "$(tail -c1 "$key_file" | wc -l)" -eq 0 ]] && printf '\n' >>"$key_file"
  chmod 600 "$key_file"

  pass="${UPDATE_SIGNING_PRIVATE_KEY_PASSWORD:-}"
  printf '%s\n' "$pass" | minisign -S -s "$key_file" -m "$archive" -x "$sig_file"
  sig_b64="$(base64 <"$sig_file" | tr -d '\n')"
fi

repo="${GITHUB_REPOSITORY:-korbindeman/ochre}"
url="https://github.com/${repo}/releases/download/${tag}/${app_name}.tar.gz"
pub_date="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
notes="${RELEASE_NOTES:-Alpha build. Expect rough edges.}"

export UPDATE_VERSION="$version"
export UPDATE_NOTES="$notes"
export UPDATE_PUB_DATE="$pub_date"
export UPDATE_URL="$url"
export UPDATE_SIG="$sig_b64"
export UPDATE_JSON_PATH="$out/latest.json"

python3 <<'PY'
import json, os
doc = {
    "version": os.environ["UPDATE_VERSION"],
    "notes": os.environ["UPDATE_NOTES"],
    "pub_date": os.environ["UPDATE_PUB_DATE"],
    "platforms": {
        "darwin-aarch64": {
            "signature": os.environ["UPDATE_SIG"],
            "url": os.environ["UPDATE_URL"],
        }
    },
}
path = os.environ["UPDATE_JSON_PATH"]
with open(path, "w", encoding="utf-8") as f:
    json.dump(doc, f, indent=2)
    f.write("\n")
print(f"Wrote {path}")
PY

ls -la "$out"
