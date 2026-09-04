#!/bin/sh
# Build a Developer ID-signed, notarized Bridge DMG for GitHub Releases.
# Requires a "Developer ID Application" identity in the keychain and Apple
# notary credentials in the environment (API key or Apple ID).
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"

if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
  identity=$(security find-identity -v -p codesigning 2>/dev/null | awk -F'"' '/Developer ID Application/ { print $2; exit }')
  if [ -n "$identity" ]; then
    export APPLE_SIGNING_IDENTITY="$identity"
  fi
fi

if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
  echo "release-dmg: no Developer ID Application identity. Install a Developer ID certificate or set APPLE_SIGNING_IDENTITY." >&2
  exit 1
fi

if [ -z "${APPLE_API_KEY_PATH:-}" ] && [ -z "${APPLE_ID:-}" ]; then
  echo "release-dmg: set APPLE_API_ISSUER + APPLE_API_KEY + APPLE_API_KEY_PATH (preferred) or APPLE_ID + APPLE_PASSWORD + APPLE_TEAM_ID for notarization." >&2
  exit 1
fi

echo "Signing as: $APPLE_SIGNING_IDENTITY"
bun run check
bun run test
bun run tauri build --bundles app,dmg

dmg=$(ls -1 "$project_root"/src-tauri/target/release/bundle/dmg/Bridge_0.5.0_*.dmg 2>/dev/null | head -n 1)
if [ -z "$dmg" ]; then
  echo "release-dmg: expected Bridge_0.5.0_*.dmg under src-tauri/target/release/bundle/dmg/" >&2
  exit 1
fi

echo "DMG: $dmg"
xcrun stapler validate "$dmg"
shasum -a 256 "$dmg"
