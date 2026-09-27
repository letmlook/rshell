#!/usr/bin/env bash
# RShell frontend + workspace verification entrypoint.
#
# Usage: ./scripts/verify.sh [--skip-install]
#
# Runs the same checks that CI consumes, in a single fail-fast script so the
# order cannot drift between humans, CI, and the documentation. The script
# always runs from the repository root and propagates child exit codes.
#
# --skip-install skips the npm ci step; use this when dependencies are already
# installed (CI caches, repeat local runs).
#
# The script intentionally does NOT print or accept signing or notary secrets.

set -euo pipefail

if (( $# > 1 )); then
    echo "Usage: $0 [--skip-install]" >&2
    exit 2
fi

skip_install=0
for arg in "$@"; do
    case "$arg" in
        --skip-install) skip_install=1 ;;
        *) echo "Unknown argument: $arg" >&2; exit 2 ;;
    esac
done

RSHELL_REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$RSHELL_REPO_ROOT"

if (( skip_install == 0 )); then
    npm ci
fi

npm run typecheck
npm test
npm run build
npm run check:docs
npm run test:scripts

RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo fmt --all --check
RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo clippy --workspace --all-targets -- -D warnings
RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo test --workspace