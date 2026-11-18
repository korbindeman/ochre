#!/usr/bin/env bash
# Run a command only after every work lease has cleared.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export BROWSER_ROOT="$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

# Wait for other editors before touching target/. Re-enter under the lease
# so cargo wipes cannot race a session that is still writing sources.
if [ "${BROWSER_WORK_GATE_INNER:-}" != 1 ]; then
  export BROWSER_WORK_GATE_INNER=1
  exec python3 "$ROOT/scripts/work-lease.py" run "$0" "$@"
fi

# After the working tree is renamed or moved, rustc dep-info `*.d` files
# still name the old /crates/ paths. Cargo's fingerprint then treats
# browser-core / browser-ffi as fresh even though rustc never ran, so the
# release dylib stays weeks stale and the Swift link fails on new FFI symbols.
#
# Cheap: a couple of file reads. Wipe only what is actually stale.
wipe_stale_cargo_after_move() {
  local profile dfile path stale
  for profile in debug release; do
    [[ -d "$ROOT/target/${profile}" ]] || continue
    stale=0
    for dfile in "$ROOT/target/${profile}"/*.d "$ROOT/target/${profile}/deps"/*.d; do
      [[ -f "$dfile" ]] || continue
      while IFS= read -r path; do
        [[ -z "$path" ]] && continue
        case "$path" in
          "$ROOT"/*) continue ;;
          */.cargo/*) continue ;;
        esac
        stale=1
        break 2
      done < <(tr ' :' '\n' < "$dfile" | grep -E '/crates/(core|ffi)/' || true)
    done
    if [[ "$stale" -eq 1 ]]; then
      echo "[build] stale cargo ${profile} dep-info after tree move; wiping workspace crate artifacts" >&2
      rm -f "$ROOT/target/${profile}"/libbrowser_ffi.* \
        "$ROOT/target/${profile}"/libbrowser_core.* \
        "$ROOT/target/${profile}"/browser "$ROOT/target/${profile}"/browser_helper \
        "$ROOT/target/${profile}"/browser.d "$ROOT/target/${profile}"/browser_helper.d
      rm -f "$ROOT/target/${profile}"/deps/libbrowser_ffi* \
        "$ROOT/target/${profile}"/deps/libbrowser_core* \
        "$ROOT/target/${profile}"/deps/browser_ffi* \
        "$ROOT/target/${profile}"/deps/browser_core* \
        "$ROOT/target/${profile}"/deps/browser-* \
        "$ROOT/target/${profile}"/deps/browser_helper-*
      rm -rf "$ROOT/target/${profile}"/.fingerprint/browser-ffi-* \
        "$ROOT/target/${profile}"/.fingerprint/browser-core-*
    fi
  done
}

wipe_stale_cargo_after_move

exec "$@"
