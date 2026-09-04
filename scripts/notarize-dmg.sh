#!/bin/sh
# Notarize and staple an already-signed Bridge DMG.
# Loads ~/.bridge-release/env when present (not in git).
set -eu

if [ -f "$HOME/.bridge-release/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.bridge-release/env"
fi

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dmg=${1:-}
if [ -z "$dmg" ]; then
  dmg=$(ls -1 "$project_root"/src-tauri/target/release/bundle/dmg/Bridge_0.5.0_*.dmg 2>/dev/null | head -n 1 || true)
fi
if [ -z "$dmg" ] || [ ! -f "$dmg" ]; then
  echo "notarize-dmg: pass a Bridge_0.5.0_*.dmg path or build one first" >&2
  exit 1
fi

if [ -z "${APPLE_API_KEY_PATH:-}" ] || [ -z "${APPLE_API_KEY:-}" ]; then
  echo "notarize-dmg: set APPLE_API_KEY + APPLE_API_KEY_PATH (and APPLE_API_ISSUER for Team keys)" >&2
  exit 1
fi

if [ ! -f "$APPLE_API_KEY_PATH" ]; then
  echo "notarize-dmg: API key not found at $APPLE_API_KEY_PATH" >&2
  exit 1
fi

echo "Notarizing $dmg"
set -- --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --wait --timeout 30m
if [ -n "${APPLE_API_ISSUER:-}" ]; then
  set -- "$@" --issuer "$APPLE_API_ISSUER"
fi
xcrun notarytool submit "$dmg" "$@"
xcrun stapler staple "$dmg"
app="$project_root/src-tauri/target/release/bundle/macos/Bridge.app"
if [ -d "$app" ]; then
  xcrun stapler staple "$app" || true
fi
xcrun stapler validate "$dmg"
shasum -a 256 "$dmg"
echo "Notarized: $dmg"
