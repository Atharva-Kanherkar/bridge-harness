#!/bin/sh
# Build, test, sign, notarize, and validate a public macOS DMG. No publishing.
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"
. "$project_root/scripts/release-common.sh"
release_load_env
release_require_identity
release_require_credentials

# A caller's Cargo target override must not make us validate a stale bundle at
# the default path after compiling somewhere else.
export CARGO_TARGET_DIR="$project_root/src-tauri/target"

# Run explicit release gates even if a developer changed the Tauri build hook.
npm run build
npm run test

app_version=$(node -p 'JSON.parse(require("fs").readFileSync("src-tauri/tauri.conf.json")).version')
app="$project_root/src-tauri/target/release/bundle/macos/Bridge.app"
release_tmp=$(mktemp -d "${TMPDIR:-/tmp}/bridge-release.XXXXXX")
cleanup() {
  release_exit_status=$?
  trap - EXIT
  # Preserve an existing development bundle if this invocation never produced
  # its replacement (for example because Cargo used a target subdirectory).
  if [ -e "$release_tmp/previous/Bridge.app" ] && [ ! -e "$app" ]; then
    mv "$release_tmp/previous/Bridge.app" "$app"
  fi
  rm -rf "$release_tmp"
  exit "$release_exit_status"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
if [ -e "$app" ]; then
  mkdir "$release_tmp/previous"
  mv "$app" "$release_tmp/previous/Bridge.app"
fi

# Use one notarization implementation for both Team/Individual API keys and
# Apple ID credentials. The built-in bundler still signs the app and sidecars.
env -u APPLE_API_KEY -u APPLE_API_KEY_PATH -u APPLE_API_ISSUER \
  -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID \
  node node_modules/@tauri-apps/cli/tauri.js build --bundles app --ci
if [ ! -d "$app" ]; then
  echo "release: this build did not produce the expected Bridge.app; check Cargo target configuration. No previous bundle will be released." >&2
  exit 1
fi
sh "$project_root/scripts/verify-macos-app.sh" "$app"

# Staple the app before putting it in the disk image, so the installed app
# carries its own offline Gatekeeper ticket too.
ditto -c -k --keepParent "$app" "$release_tmp/Bridge.zip"
release_notarize "$release_tmp/Bridge.zip" "$release_tmp/app-notary.json"
xcrun stapler staple "$app"
xcrun stapler validate "$app"
spctl --assess --type execute --verbose=2 "$app"

arch=$(uname -m)
case "$arch" in
  arm64) dmg_arch=aarch64 ;;
  x86_64) dmg_arch=x64 ;;
  *) echo "release: unsupported macOS architecture: $arch" >&2; exit 1 ;;
esac
mkdir -p "$release_tmp/image" "$project_root/src-tauri/target/release/bundle/dmg"
ditto "$app" "$release_tmp/image/Bridge.app"
ln -s /Applications "$release_tmp/image/Applications"
# Build at a fresh path: an old matching version must never be selected by mtime.
hdiutil create -volname Bridge -srcfolder "$release_tmp/image" -format UDZO -ov "$release_tmp/Bridge.dmg"
codesign --force --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$release_tmp/Bridge.dmg"
sh "$project_root/scripts/notarize-dmg.sh" "$release_tmp/Bridge.dmg"

dmg="$project_root/src-tauri/target/release/bundle/dmg/Bridge_${app_version}_${dmg_arch}.dmg"
mv "$release_tmp/Bridge.dmg" "$dmg"
(cd "$(dirname -- "$dmg")" && shasum -a 256 "$(basename -- "$dmg")") > "$dmg.sha256"
echo "Verified public release: $dmg"
