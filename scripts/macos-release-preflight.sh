#!/usr/bin/env bash
# RShell macOS release preflight.
#
# Usage: ./scripts/macos-release-preflight.sh [--unsigned] [app-path]
#   app-path defaults to src-tauri/target/release/bundle/macos/RShell.app.
#
# Without --unsigned, the script requires APPLE_SIGNING_IDENTITY and
# APPLE_NOTARY_PROFILE to be set and refuses to print or echo either value.
# With --unsigned, both are allowed to be absent (the preflight still
# confirms the app exists, the Bundle ID, and the bundle layout).
#
# The script always runs from the repository root and propagates child exit
# codes unchanged.

set -euo pipefail

if [[ $# -gt 2 ]]; then
    echo "Usage: $0 [--unsigned] [app-path]" >&2
    exit 2
fi

unsigned=0
positional=()
for arg in "$@"; do
    case "$arg" in
        --unsigned) unsigned=1 ;;
        -h|--help)
            echo "Usage: $0 [--unsigned] [app-path]" >&2
            exit 0
            ;;
        --*)
            echo "Unknown flag: $arg" >&2
            exit 2
            ;;
        *) positional+=("$arg") ;;
    esac
done

RSHELL_REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$RSHELL_REPO_ROOT"

app_path="${positional[0]:-$RSHELL_REPO_ROOT/src-tauri/target/release/bundle/macos/RShell.app}"
app_name="$(basename "$app_path")"
bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app_path/Contents/Info.plist" 2>/dev/null || true)"

if [[ ! -d "$app_path" ]]; then
    echo "App bundle not found at $app_path" >&2
    echo "Build it first with: RUSTUP_TOOLCHAIN=stable npm run tauri:build -- --bundles app" >&2
    exit 1
fi

if [[ "$bundle_id" != "com.letmlook.rshell" ]]; then
    echo "Unexpected Bundle ID: $bundle_id" >&2
    echo "Expected com.letmlook.rshell" >&2
    exit 1
fi

# Never echo or store the actual values; only report their presence.
have_identity=0
have_profile=0
if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then have_identity=1; fi
if [[ -n "${APPLE_NOTARY_PROFILE:-}" ]]; then have_profile=1; fi

if (( unsigned == 1 )); then
    echo "Preflight (unsigned): app=$app_name bundle-id=$bundle_id" >&2
    if (( have_identity || have_profile )); then
        echo "Refusing to mix --unsigned with signing/notary credentials." >&2
        exit 1
    fi
    exit 0
fi

if (( have_identity == 0 || have_profile == 0 )); then
    echo "APPLE_SIGNING_IDENTITY and APPLE_NOTARY_PROFILE must be set for signed preflight." >&2
    echo "Re-run with --unsigned for local preflight that skips signing and notarization." >&2
    exit 1
fi

echo "Preflight (signed): app=$app_name bundle-id=$bundle_id" >&2
codesign --verify --deep --strict --verbose=2 "$app_path"
xcrun notarytool submit "$app_path" --keychain-profile "$APPLE_NOTARY_PROFILE" --wait --no-progress
xcrun stapler staple "$app_path"
spctl --assess --type execute --verbose=2 "$app_path"