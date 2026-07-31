#!/usr/bin/env bash
# Build rshell release binary for the current host (Linux or macOS).
#
# Usage:  ./scripts/build.sh [target]
#   target (optional): triple, e.g. x86_64-unknown-linux-gnu.
#                     If omitted, builds for the host triple.
#
# Output: target/release/<OS>-<ARCH>/rshell-<version>
#         plus a "latest" symlink at target/release/<OS>-<ARCH>/rshell
#
# Required: cargo, git, rustc. The .git/ directory must be present so
#          we can read the version via `git describe`. If absent, we
#          fall back to the workspace.package.version from Cargo.toml.

set -euo pipefail

# ---------- 1. workspace metadata ----------
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="$(grep -E '^version[[:space:]]*=' src-tauri/Cargo.toml | head -1 | sed -E 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')"
if [[ -z "$VERSION" ]]; then
    echo "ERROR: cannot read version from src-tauri/Cargo.toml" >&2
    exit 1
fi

# git describe fallback: if a tag is present, prefer it
if command -v git >/dev/null 2>&1 && [[ -d .git ]]; then
    if GIT_DESC="$(git describe --tags --always --dirty='-dev' 2>/dev/null)"; then
        # Only use git describe when it actually mentions a version (e.g. v0.1.0-2-gabcdef)
        if [[ "$GIT_DESC" =~ ^[0-9]+\.[0-9]+\.[0-9]+ || "$GIT_DESC" =~ ^v[0-9]+\.[0-9]+\.[0-9]+ ]]; then
            # strip leading 'v' for filename compatibility
            VERSION="$(echo "$GIT_DESC" | sed -E 's/^v//')"
        fi
    fi
fi

# ---------- 2. host triple detection ----------
HOST_TRIPLE="${1:-}"
if [[ -z "$HOST_TRIPLE" ]]; then
    if ! HOST_TRIPLE="$(rustc -vV 2>/dev/null | awk '/^host: /{print $2}')"; then
        echo "ERROR: cannot determine host triple; pass it as the first arg" >&2
        exit 1
    fi
fi

# Normalize OS / ARCH parts for the output directory name
OS_PART="$(echo "$HOST_TRIPLE" | awk -F'-' '{print $1}')"
ARCH_PART="$(echo "$HOST_TRIPLE" | awk -F'-' '{for (i=2; i<=NF; i++) if ($i != "unknown" && $i != "gnu" && $i != "musl" && $i != "apple") print $i; exit}')"
# OS detection by HOST_TRIPLE (Git Bash uname lies on Windows, returns "Linux")
case "$HOST_TRIPLE" in
    *-pc-windows-msvc|*-pc-windows-gnu) OS_LABEL="windows" ;;
    *apple-darwin*)                       OS_LABEL="macos" ;;
    *linux-gnu*|*-linux-musl*|*-unknown-linux-*) OS_LABEL="linux" ;;
    *) OS_LABEL="$OS_PART" ;;
esac
ARCH_LABEL="$(uname -m 2>/dev/null || echo "$ARCH_PART")"

OUT_DIR="$REPO_ROOT/target/release/$OS_LABEL-$ARCH_LABEL"
BIN_NAME="rshell"
EXT=""
case "$OS_LABEL" in
    windows*) BIN_NAME="rshell.exe"; EXT=".exe" ;;
esac
case "$OS_LABEL" in
    windows*) OUT_BIN="$OUT_DIR/${BIN_NAME%.exe}-${VERSION}.exe" ;;
    *)        OUT_BIN="$OUT_DIR/${BIN_NAME}-${VERSION}${EXT}" ;;
esac
LATEST_BIN="$OUT_DIR/${BIN_NAME}"

# ---------- 3. ensure target dir ----------
mkdir -p "$OUT_DIR"

# ---------- 4. build ----------
echo "Building rshell ${VERSION} for ${HOST_TRIPLE} -> ${OUT_BIN}"
cargo build --release --locked

# ---------- 5. copy + latest symlink ----------
# Find the actual binary cargo produced (lives in target/release/, may be in subdirs)
SRC_BIN="$(find "$REPO_ROOT/target/release" -maxdepth 2 -name 'rshell' -o -name 'rshell.exe' 2>/dev/null | head -1)"
if [[ -z "$SRC_BIN" ]]; then
    echo "ERROR: built binary not found under target/release/" >&2
    exit 1
fi
cp -f "$SRC_BIN" "$OUT_BIN"
ln -sfn "$(basename "$OUT_BIN")" "$LATEST_BIN"

# ---------- 6. report ----------
echo
echo "Done."
echo "  Binary : $OUT_BIN"
echo "  Latest : $LATEST_BIN"
ls -lh "$OUT_BIN"
