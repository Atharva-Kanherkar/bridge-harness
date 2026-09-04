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

app="$project_root/src-tauri/target/release/bundle/macos/Bridge.app"
sdk="$app/Contents/Resources/sidecar/claude-agent/node_modules/@anthropic-ai/claude-agent-sdk/package.json"
if [ ! -f "$sdk" ]; then
  echo "release-dmg: bundled app is missing the Claude Agent SDK at $sdk" >&2
  exit 1
fi
if ! codesign -dv --verbose=2 "$app" 2>&1 | grep -q 'Authority=Developer ID Application'; then
  echo "release-dmg: $app is not Developer ID-signed" >&2
  codesign -dv --verbose=2 "$app" >&2 || true
  exit 1
fi

echo "DMG: $dmg"
if [ -f "$HOME/.bridge-release/env" ] && [ -z "${APPLE_API_KEY_PATH:-}" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.bridge-release/env"
fi
sh "$project_root/scripts/notarize-dmg.sh" "$dmg"
