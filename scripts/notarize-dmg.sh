#!/bin/sh
# Notarize and verify this exact DMG and the app inside it. No publishing.
set -eu
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$project_root/scripts/release-common.sh"
release_load_env
release_require_credentials

dmg=${1:-}
if [ -z "$dmg" ] || [ ! -f "$dmg" ]; then
  echo "notarize-dmg: pass the exact signed DMG path to notarize." >&2
  exit 1
fi
dmg=$(CDPATH= cd -- "$(dirname -- "$dmg")" && pwd)/$(basename -- "$dmg")
release_tmp=$(mktemp -d "${TMPDIR:-/tmp}/bridge-notary.XXXXXX")
mount_path="$release_tmp/mount"
cleanup() {
  if [ -d "$mount_path" ]; then
    if ! hdiutil detach "$mount_path" -quiet; then
      echo "notarize-dmg: could not detach $mount_path; retained the temporary directory." >&2
      return
    fi
  fi
  rm -rf "$release_tmp"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
codesign --verify --strict "$dmg"
codesign -dv --verbose=4 "$dmg" 2> "$release_tmp/signature.txt"
if ! grep -q '^Authority=Developer ID Application:' "$release_tmp/signature.txt"; then
  echo "notarize-dmg: the DMG is not signed with Developer ID Application." >&2
  exit 1
fi

hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$mount_path" -quiet
sh "$project_root/scripts/verify-macos-app.sh" "$mount_path/Bridge.app"
# A ticket on the enclosing DMG does not staple the app inside it.
xcrun stapler validate "$mount_path/Bridge.app"
spctl --assess --type execute --verbose=2 "$mount_path/Bridge.app"
hdiutil detach "$mount_path" -quiet
rmdir "$mount_path" 2>/dev/null || true

release_notarize "$dmg" "$release_tmp/dmg-notary.json"
xcrun stapler staple "$dmg"
xcrun stapler validate "$dmg"
spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
shasum -a 256 "$dmg"
echo "Notarized and verified: $dmg"
