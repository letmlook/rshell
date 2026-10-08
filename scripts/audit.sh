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

# 已审阅的例外逐条记录在 scripts/audit-ignored.txt，并逐条回显到 stderr，
# 使 CI 日志里能看到「哪些告警被豁免」。文件缺失视为仓库配置错误而不是
# 「没有例外」——否则删掉文件就等于悄悄关掉整道门禁。
RSHELL_AUDIT_IGNORE_FILE="$RSHELL_REPO_ROOT/scripts/audit-ignored.txt"
if [[ ! -f "$RSHELL_AUDIT_IGNORE_FILE" ]]; then
    echo "missing $RSHELL_AUDIT_IGNORE_FILE; reviewed cargo-audit exceptions cannot be verified." >&2
    exit 1
fi

audit_ignore_args=()
while IFS= read -r raw_line || [[ -n "$raw_line" ]]; do
    line="${raw_line%%#*}"
    line="${line//[[:space:]]/}"
    if [[ -z "$line" ]]; then
        continue
    fi
    audit_ignore_args+=(--ignore "$line")
    echo "cargo audit: reviewed exception $line (see scripts/audit-ignored.txt)" >&2
done < "$RSHELL_AUDIT_IGNORE_FILE"

npm audit --registry=https://registry.npmjs.org

# ${arr[@]+...} keeps an empty list safe under `set -u` on bash 3.2 (macOS runners).
RUSTUP_TOOLCHAIN=stable RUSTUP_NO_UPDATE_CHECK=1 cargo audit ${audit_ignore_args[@]+"${audit_ignore_args[@]}"} --file src-tauri/Cargo.lock