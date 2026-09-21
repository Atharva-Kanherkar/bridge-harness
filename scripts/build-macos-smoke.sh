#!/bin/sh
# Build the packaged app used by routine macOS CI without release credentials.
# The dedicated Tauri profile ad-hoc signs the bundle and disables updater
# artifacts; public Developer ID signing and notarization stay release-only.
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "build-macos-smoke: macOS is required." >&2
  exit 1
fi

# Never let a developer shell or future CI environment turn this into a release
# build. The committed smoke profile supplies the credential-free `-` identity.
unset APPLE_CERTIFICATE
unset APPLE_CERTIFICATE_PASSWORD
unset APPLE_SIGNING_IDENTITY
unset APPLE_API_ISSUER
unset APPLE_API_KEY
unset APPLE_API_KEY_PATH
unset APPLE_API_KEY_P8
unset APPLE_ID
unset APPLE_PASSWORD
unset APPLE_TEAM_ID
unset BRIDGE_RELEASE_ENV
unset TAURI_SIGNING_PRIVATE_KEY
unset TAURI_SIGNING_PRIVATE_KEY_PATH
unset TAURI_SIGNING_PRIVATE_KEY_PASSWORD
unset TAURI_PRIVATE_KEY
unset TAURI_PRIVATE_KEY_PATH
unset TAURI_PRIVATE_KEY_PASSWORD

# Fix the output path so the smoke harness can never pick up a successful build
# routed to another Cargo target directory while a stale default bundle remains.
export CARGO_TARGET_DIR="$project_root/src-tauri/target"

node "$project_root/node_modules/@tauri-apps/cli/tauri.js" build \
  --bundles app \
  --config "$project_root/src-tauri/tauri.macos-smoke.conf.json" \
  --ci

app="$project_root/src-tauri/target/release/bundle/macos/Bridge.app"
if [ ! -d "$app/Contents" ]; then
  echo "build-macos-smoke: expected packaged app at $app" >&2
  exit 1
fi

signature=$(mktemp "${TMPDIR:-/tmp}/bridge-smoke-signature.XXXXXX")
cleanup() {
  rm -f "$signature"
}
trap cleanup EXIT HUP INT TERM

codesign --verify --deep --strict --verbose=2 "$app"
codesign -dv --verbose=4 "$app" 2> "$signature"
if ! grep -Fxq "Signature=adhoc" "$signature"; then
  echo "build-macos-smoke: Bridge.app is not credential-free ad-hoc signed." >&2
  cat "$signature" >&2
  exit 1
fi
if grep -q '^Authority=Developer ID Application:' "$signature"; then
  echo "build-macos-smoke: Bridge.app unexpectedly used a Developer ID identity." >&2
  exit 1
fi

echo "Built credential-free macOS smoke bundle: $app"
