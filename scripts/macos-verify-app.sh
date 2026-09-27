#!/usr/bin/env bash
# RShell macOS app verification helper.
#
# Usage: ./scripts/macos-verify-app.sh <app-path>
#
# Verifies the Bundle ID, code-signing status, and Gatekeeper assessment
# of an existing .app bundle. The script does not perform signing or
# notarization; it only inspects a bundle that has already been produced
# by the regular tauri:build workflow.
#
# Exit codes:
#   0 - all checks passed
#   1 - one or more checks failed (with a per-check reason on stderr)
#   2 - usage error
#
# The script never prints signing or notary secrets.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <app-path>" >&2
    exit 2
fi

app_path="$1"

if [[ ! -d "$app_path" ]]; then
    echo "App bundle not found: $app_path" >&2
    exit 1
fi

if [[ ! -f "$app_path/Contents/Info.plist" ]]; then
    echo "Missing Contents/Info.plist in $app_path" >&2
    exit 1
fi

bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app_path/Contents/Info.plist")"
if [[ "$bundle_id" != "com.letmlook.rshell" ]]; then
    echo "Unexpected Bundle ID: $bundle_id (expected com.letmlook.rshell)" >&2
    exit 1
fi

echo "Verified Bundle ID: $bundle_id" >&2
codesign --verify --deep --strict --verbose=2 "$app_path"
echo "Code signature strict verification passed." >&2

# Gatekeeper assessment fails for unsigned debug bundles; treat that as a
# non-fatal diagnostic rather than a hard failure so the script still
# produces useful output for local debug runs.
if spctl --assess --type execute --verbose=2 "$app_path"; then
    echo "Gatekeeper assessment passed." >&2
    exit 0
fi

echo "Gatekeeper rejected the bundle (unsigned or un-notarized)." >&2
exit 1