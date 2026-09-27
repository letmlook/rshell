#!/usr/bin/env bash
# RShell dependency audit entrypoint.
#
# Usage: ./scripts/audit.sh
#
# Runs npm audit against the official registry (npmmirror does not implement
# the audit API) and cargo audit against src-tauri/Cargo.lock. The script
# always runs from the repository root, propagates child exit codes, and
# refuses to proceed when cargo-audit is not installed.
#
# The script intentionally does NOT print or accept signing or notary secrets.

set -euo pipefail

RSHELL_REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$RSHELL_REPO_ROOT"

if ! command -v cargo-audit >/dev/null 2>&1; then
    echo "cargo-audit is required for scripts/audit.sh but was not found on PATH." >&2
    echo "Install with: cargo install cargo-audit --version 0.22.2 --locked" >&2
    exit 1
fi

npm audit --registry=https://registry.npmjs.org

RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo audit --file src-tauri/Cargo.lock