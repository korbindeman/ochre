#!/usr/bin/env bash
# Print the path to (and optionally the contents of) the latest local crash payload.
set -euo pipefail

support="${HOME}/Library/Application Support"
if [[ -f "${support}/Browser Dev/crashes/latest.json" ]]; then
    latest="${support}/Browser Dev/crashes/latest.json"
elif [[ -f "${support}/Browser/crashes/latest.json" ]]; then
    latest="${support}/Browser/crashes/latest.json"
else
    echo "No crash payload found." >&2
    echo "Expected: ~/Library/Application Support/Browser Dev/crashes/latest.json" >&2
    exit 1
fi

if [[ "${1:-}" == "--print" ]]; then
    cat "${latest}"
else
    echo "${latest}"
fi
