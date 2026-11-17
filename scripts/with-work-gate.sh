#!/usr/bin/env bash
# Run a command only after every work lease has cleared.
#
# Also the single pre-build hook where the codec-enabled CEF is resolved
# (ADR-0040/0042): every build recipe funnels through here, so this is where we
# point cef-dll-sys at the custom build. If the private ochre_cef artifact is
# reachable (scripts/fetch-cef.sh prints its path), CEF_PATH is exported and the
# build embeds proprietary codecs (H.264/AAC). If not - a contributor without
# repo access - nothing is set and cef-dll-sys downloads the stock codec-less
# prebuilt (ADR-0042's retreat). Set CEF_PATH yourself to override; set
# OCHRE_NO_CODECS=1 to force the stock build.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export BROWSER_ROOT="$ROOT"
export PATH="$HOME/.cargo/bin:$PATH"

if [ -z "${CEF_PATH:-}" ] && [ -z "${OCHRE_NO_CODECS:-}" ]; then
  cef="$("$ROOT/scripts/fetch-cef.sh" 2>/dev/null || true)"
  [ -n "$cef" ] && export CEF_PATH="$cef"
fi

# Wait for other editors before touching target/. Re-enter under the lease
# so cmake/cargo wipes cannot race a session that is still writing sources.
if [ "${BROWSER_WORK_GATE_INNER:-}" != 1 ]; then
  export BROWSER_WORK_GATE_INNER=1
  exec python3 "$ROOT/scripts/work-lease.py" run "$0" "$@"
fi

# After the working tree is renamed or moved, two caches keep the old
# absolute path and break the next build:
#
#   1. CMakeCache.txt / build.ninja (cef-dll-sys): cmake refuses to
#      reconfigure ("CMakeCache.txt directory is different than the
#      directory where CMakeCache.txt was created").
#   2. rustc dep-info `*.d` files: they still name the old /crates/ paths.
#      Cargo's fingerprint then treats browser-core / browser-ffi as fresh
#      even though rustc never ran, so the release dylib stays weeks stale
#      and the Swift link fails on new FFI symbols.
#
# Cheap: a couple of file reads. Wipe only what is actually stale.
wipe_stale_cef_cmake() {
  local profile dir cache ninja cached_dir actual workdir
  for profile in debug release; do
    for dir in "$ROOT/target/${profile}/build"/cef-dll-sys-*; do
      [[ -d "$dir/out/build" ]] || continue
      actual="$(cd "$dir/out/build" && pwd)"
      cache="$dir/out/build/CMakeCache.txt"
      ninja="$dir/out/build/build.ninja"
      if [[ -f "$cache" ]]; then
        cached_dir="$(awk -F= '/^CMAKE_CACHEFILE_DIR:INTERNAL=/{print $2; exit}' "$cache")"
        if [[ -n "$cached_dir" && "$cached_dir" != "$actual" ]]; then
          echo "[cef] stale CMake cache ($cached_dir); wiping $dir" >&2
          rm -rf "$dir"
          continue
        fi
      fi
      if [[ -f "$ninja" ]]; then
        workdir="$(awk -F'= ' '/^cmake_ninja_workdir/{print $2; exit}' "$ninja")"
        workdir="${workdir%/}"
        if [[ -n "$workdir" && "$workdir" != "$actual" ]]; then
          echo "[cef] stale Ninja workdir ($workdir); wiping $dir" >&2
          rm -rf "$dir"
        fi
      fi
    done
  done
}

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
      echo "[cef] stale cargo ${profile} dep-info after tree move; wiping workspace crate artifacts" >&2
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

wipe_stale_cef_cmake
wipe_stale_cargo_after_move

exec "$@"
