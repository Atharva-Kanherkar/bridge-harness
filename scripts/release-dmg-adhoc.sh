#!/bin/sh
# Build an ad-hoc signed DMG to hand to people directly, for when no Developer
# ID or notarization credentials are available. macOS will not trust it until
# the person opens it once through Privacy & Security, or clears quarantine.
# No Developer ID, no notarization, no updater artifacts, no publishing.
# The public release path stays scripts/release-dmg.sh.
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"

if [ "$(uname -s)" != "Darwin" ]; then
  echo "release-dmg-adhoc: macOS is required." >&2
  exit 1
fi

case "$(uname -m)" in
  arm64) dmg_arch=aarch64 ;;
  x86_64) dmg_arch=x64 ;;
  *) echo "release-dmg-adhoc: unsupported macOS architecture: $(uname -m)" >&2; exit 1 ;;
esac

# The smoke build clears every signing and updater credential, uses the
# committed ad-hoc profile, and fails unless the bundle is ad-hoc signed.
sh "$project_root/scripts/build-macos-smoke.sh"

app="$project_root/src-tauri/target/release/bundle/macos/Bridge.app"
app_version=$(node -p 'JSON.parse(require("fs").readFileSync("src-tauri/tauri.conf.json")).version')
bundle_version=$(plutil -extract CFBundleShortVersionString raw -o - "$app/Contents/Info.plist")
if [ "$bundle_version" != "$app_version" ]; then
  echo "release-dmg-adhoc: Bridge.app is $bundle_version but this checkout is $app_version." >&2
  exit 1
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/bridge-adhoc-dmg.XXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM

mkdir "$work/image"
ditto "$app" "$work/image/Bridge.app"
ln -s /Applications "$work/image/Applications"
cat > "$work/image/How to open Bridge.txt" <<'EOF'
This copy of Bridge is not notarized by Apple, so macOS blocks the first launch.

1. Drag Bridge into Applications. Do not run it from the disk image,
   Downloads or Documents.
2. Open Terminal and run:

   /usr/bin/xattr -cr /Applications/Bridge.app

3. Open Bridge from Applications.

Without Terminal: open Bridge once, dismiss the warning, then go to
System Settings > Privacy & Security and click Open Anyway.

Claude needs Node 18 or later on PATH.
EOF

# Build at a fresh path so an older DMG is never picked up by mistake.
hdiutil create -volname Bridge -srcfolder "$work/image" -format UDZO -ov "$work/Bridge.dmg" >/dev/null
hdiutil verify "$work/Bridge.dmg" >/dev/null

out_dir="$project_root/src-tauri/target/release/bundle/dmg"
mkdir -p "$out_dir"
dmg="$out_dir/Bridge_${app_version}_${dmg_arch}_adhoc.dmg"
mv "$work/Bridge.dmg" "$dmg"
(cd "$out_dir" && shasum -a 256 "$(basename -- "$dmg")") > "$dmg.sha256"
echo "Built ad-hoc DMG (not notarized): $dmg"
