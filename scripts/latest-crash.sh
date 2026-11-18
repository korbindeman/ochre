#!/usr/bin/env bash
# Print the path to (and optionally the contents of) the newest local crash
# artifact. Checks debug and release support dirs from product.toml (plus
# legacy names), including:
#   - latest.json / dated *-panic|signal|swift|system.json
#   - signal-pending.json (async-signal-safe dump)
#   - harvested copies under crashes/ips/*.ips
# Falls back to macOS DiagnosticReports if nothing is in Application Support.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
support="${HOME}/Library/Application Support"
diag="${HOME}/Library/Logs/DiagnosticReports"

candidates=()

collect_from_crashes_dir() {
    local dir="$1"
    [[ -d "${dir}" ]] || return 0
    # Prefer explicit latest pointer and signal-pending, then dated payloads.
    [[ -f "${dir}/latest.json" ]] && candidates+=("${dir}/latest.json")
    [[ -f "${dir}/signal-pending.json" ]] && candidates+=("${dir}/signal-pending.json")
    local f
    for f in "${dir}"/*-panic.json "${dir}"/*-signal.json \
             "${dir}"/*-swift.json "${dir}"/*-system.json; do
        if [[ -f "${f}" ]]; then candidates+=("${f}"); fi
    done
    if [[ -d "${dir}/ips" ]]; then
        for f in "${dir}/ips"/*.ips; do
            if [[ -f "${f}" ]]; then candidates+=("${f}"); fi
        done
    fi
}

profile_dirs=()
while IFS= read -r dir; do
    [[ -n "${dir}" ]] && profile_dirs+=("${dir}")
done < <(python3 "${ROOT}/scripts/product.py" profile-dirs)

for dir in "${profile_dirs[@]}"; do
    collect_from_crashes_dir "${support}/${dir}/crashes"
done

ips_prefixes=()
while IFS= read -r prefix; do
    [[ -n "${prefix}" ]] && ips_prefixes+=("${prefix}")
done < <(python3 "${ROOT}/scripts/product.py" ips-prefixes)

if [[ -d "${diag}" ]]; then
    for prefix in "${ips_prefixes[@]}"; do
        for f in "${diag}/${prefix}"-*.ips "${diag}/Retired/${prefix}"-*.ips; do
            if [[ -f "${f}" ]]; then candidates+=("${f}"); fi
        done
    done
fi

expected_paths() {
    for dir in "${profile_dirs[@]}"; do
        echo "  ${support}/${dir}/crashes/"
    done
    for prefix in "${ips_prefixes[@]}"; do
        echo "  ${diag}/${prefix}-*.ips"
    done
}

if [[ ${#candidates[@]} -eq 0 ]]; then
    echo "No crash payload found." >&2
    echo "Expected under:" >&2
    expected_paths >&2
    exit 1
fi

# Newest by mtime.
latest="$(ls -t "${candidates[@]}" 2>/dev/null | head -1)"

if [[ -z "${latest}" ]]; then
    echo "No crash payload found." >&2
    exit 1
fi

case "${1:-}" in
    --print)
        echo "# ${latest}"
        cat "${latest}"
        # If latest is a harvested system summary, also print the ips path field.
        if [[ "${latest}" == *.json ]] && command -v python3 >/dev/null 2>&1; then
            ips="$(python3 -c "
import json,sys
try:
    d=json.load(open(sys.argv[1]))
    p=d.get('system_report_path') or ''
    if p: print(p)
except Exception:
    pass
" "${latest}" 2>/dev/null || true)"
            if [[ -n "${ips}" && -f "${ips}" ]]; then
                echo ""
                echo "# linked system report: ${ips}"
                # Full IPS can be huge; print a short head for the terminal.
                head -c 12000 "${ips}"
                echo ""
                echo "# … truncated; full file: ${ips}"
            fi
        fi
        ;;
    --path|"")
        echo "${latest}"
        ;;
    --list)
        ls -lt "${candidates[@]}" 2>/dev/null | head -20
        ;;
    *)
        echo "usage: $0 [--print|--path|--list]" >&2
        exit 2
        ;;
esac
