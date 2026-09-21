#!/bin/sh
# Mount the exact release DMG read-only and reuse the credential-free #555 app/daemon smoke.
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: $0 /absolute/path/to/Bridge.dmg" >&2
  exit 2
fi

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
case "$1" in
  /*) dmg=$1 ;;
  *) dmg=$(CDPATH= cd -- "$(dirname -- "$1")" && pwd)/$(basename -- "$1") ;;
esac
if [ ! -f "$dmg" ] || [ ! -f "$dmg.sha256" ]; then
  echo "release smoke: expected a DMG and adjacent .sha256 file: $dmg" >&2
  exit 1
fi

(cd "$(dirname -- "$dmg")" && shasum -a 256 -c "$(basename -- "$dmg").sha256")

mount_dir=$(mktemp -d "${TMPDIR:-/tmp}/bridge-release-smoke.XXXXXX")
mounted=false
cleanup() {
  smoke_status=$?
  trap - EXIT
  if [ "$mounted" = true ]; then
    hdiutil detach "$mount_dir" >/dev/null 2>&1 || true
  fi
  rmdir "$mount_dir" >/dev/null 2>&1 || true
  exit "$smoke_status"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$mount_dir" >/dev/null
mounted=true
if [ ! -d "$mount_dir/Bridge.app" ]; then
  echo "release smoke: mounted DMG does not contain Bridge.app" >&2
  exit 1
fi
PYTHONDONTWRITEBYTECODE=1 python3 "$project_root/scripts/smoke-packaged-macos-app.py" "$mount_dir/Bridge.app"
hdiutil detach "$mount_dir" >/dev/null
mounted=false
rmdir "$mount_dir"
trap - EXIT
