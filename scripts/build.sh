#!/usr/bin/env bash
# macOS release wrapper. Install dependencies with npm ci first.
# Usage: ./scripts/build.sh [target-triple]
# Uses the same Tauri frontend + native bundle build as npm run tauri:build.
set -euo pipefail

if (( $# > 1 )); then
    echo "Usage: $0 [target-triple]" >&2
    exit 2
fi

RSHELL_REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$RSHELL_REPO_ROOT"

if (( $# == 1 )); then
    exec npm run tauri:build -- --target "$1"
fi
exec npm run tauri:build --
