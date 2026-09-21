#!/bin/sh
# Upload one verified artifact set to Release Please's draft, then make it public.
set -eu

for name in RELEASE_TAG RELEASE_VERSION RELEASE_SHA RELEASE_ARTIFACT_DIR GITHUB_REPOSITORY; do
  eval "value=\${$name:-}"
  if [ -z "$value" ]; then
    echo "publish release: missing $name" >&2
    exit 1
  fi
done

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
artifact_dir=$(CDPATH= cd -- "$RELEASE_ARTIFACT_DIR" && pwd)
cd "$project_root"

node scripts/release-version.mjs
checkout_version=$(node -p 'JSON.parse(require("fs").readFileSync("package.json")).version')
if [ "$RELEASE_VERSION" != "$checkout_version" ] || [ "$RELEASE_TAG" != "v$RELEASE_VERSION" ]; then
  echo "publish release: checkout version, release version, and tag do not match" >&2
  exit 1
fi
if [ "$(git rev-parse HEAD)" != "$RELEASE_SHA" ] || [ "$(git rev-list -n 1 "refs/tags/$RELEASE_TAG")" != "$RELEASE_SHA" ]; then
  echo "publish release: checkout, tag, and approved release SHA do not match" >&2
  exit 1
fi
newest_tag=$(git tag --list 'v[0-9]*.[0-9]*.[0-9]*' --sort=-version:refname | head -n 1)
if [ "$newest_tag" != "$RELEASE_TAG" ]; then
  echo "publish release: $RELEASE_TAG is superseded by newer version tag ${newest_tag:-<none>}" >&2
  exit 1
fi

set -- "$artifact_dir"/Bridge_"$RELEASE_VERSION"_*.dmg
if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
  echo "publish release: expected exactly one versioned DMG" >&2
  exit 1
fi
dmg=$1
dmg_name=$(basename -- "$dmg")
arch=${dmg_name#Bridge_"$RELEASE_VERSION"_}
arch=${arch%.dmg}
case "$arch" in
  aarch64|x64) ;;
  *) echo "publish release: unsupported DMG architecture $arch" >&2; exit 1 ;;
esac
updater_tar="$artifact_dir/Bridge_${RELEASE_VERSION}_${arch}.app.tar.gz"
updater_sig="$updater_tar.sig"
for file in "$dmg.sha256" "$updater_tar" "$updater_sig"; do
  if [ ! -f "$file" ]; then
    echo "publish release: missing verified artifact $file" >&2
    exit 1
  fi
done
(cd "$artifact_dir" && shasum -a 256 -c "$dmg_name.sha256")
node scripts/verify-updater-signature.mjs \
  src-tauri/tauri.conf.json "$updater_tar" "$updater_sig"

notes_file=$(mktemp "${TMPDIR:-/tmp}/bridge-release-notes.XXXXXX")
assets_file=$(mktemp "${TMPDIR:-/tmp}/bridge-release-assets.XXXXXX")
cleanup() {
  publish_status=$?
  trap - EXIT
  rm -f "$notes_file" "$assets_file"
  exit "$publish_status"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

fetch_remote_assets() {
  gh release view "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" --json assets --jq '.assets[].name' > "$assets_file"
}

require_exact_remote_assets() {
  asset_count=$(awk 'NF { count += 1 } END { print count + 0 }' "$assets_file")
  if [ "$asset_count" -ne 5 ]; then
    echo "publish release: expected exactly five release assets, found $asset_count" >&2
    return 1
  fi
  for name in "$dmg_name" "$dmg_name.sha256" "$(basename -- "$updater_tar")" "$(basename -- "$updater_sig")" latest.json; do
    if ! grep -F -x "$name" "$assets_file" >/dev/null; then
      echo "publish release: uploaded asset is not visible: $name" >&2
      return 1
    fi
  done
}

release_is_draft=$(gh release view "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" --json isDraft --jq .isDraft)
if [ "$release_is_draft" = false ]; then
  fetch_remote_assets
  require_exact_remote_assets
  echo "$RELEASE_TAG is already public with the complete verified asset set."
  trap - EXIT
  rm -f "$notes_file" "$assets_file"
  exit 0
fi
if [ "$release_is_draft" != true ]; then
  echo "publish release: could not determine whether $RELEASE_TAG is a draft" >&2
  exit 1
fi

gh release view "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" --json body --jq .body > "$notes_file"
pub_date=$(date -u +%Y-%m-%dT%H:%M:%SZ)
node scripts/create-updater-manifest.mjs \
  --version "$RELEASE_VERSION" \
  --tag "$RELEASE_TAG" \
  --arch "$arch" \
  --signature-file "$updater_sig" \
  --repository "$GITHUB_REPOSITORY" \
  --updater-name "$(basename -- "$updater_tar")" \
  --notes-file "$notes_file" \
  --pub-date "$pub_date" \
  --output "$artifact_dir/latest.json"

gh release upload "$RELEASE_TAG" \
  "$dmg" "$dmg.sha256" "$updater_tar" "$updater_sig" "$artifact_dir/latest.json" \
  --repo "$GITHUB_REPOSITORY" --clobber

fetch_remote_assets
require_exact_remote_assets

# Publication is deliberately last. Any earlier failure leaves a retryable draft.
gh release edit "$RELEASE_TAG" --repo "$GITHUB_REPOSITORY" --draft=false --latest
trap - EXIT
rm -f "$notes_file" "$assets_file"
