#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
source_dir="$project_root/safari-extension/Resources"
output_dir="${BRIDGE_SAFARI_OUTPUT:-$project_root/.generated/safari-extension}"
stage_dir=$(mktemp -d)
trap 'rm -rf "$stage_dir"' EXIT

if ! command -v xcrun >/dev/null 2>&1; then
  echo "xcrun is required to build the Safari Web Extension" >&2
  exit 1
fi
if ! xcrun --find safari-web-extension-converter >/dev/null 2>&1; then
  echo "A full Xcode installation with safari-web-extension-converter is required" >&2
  exit 1
fi

cp "$source_dir/manifest.json" "$stage_dir/manifest.json"
cp "$source_dir/background.js" "$stage_dir/background.js"
cp "$project_root/browser-extension/content.js" "$stage_dir/content.js"
cp "$project_root/browser-extension/popup.html" "$stage_dir/popup.html"
cp "$project_root/browser-extension/popup.css" "$stage_dir/popup.css"
cp "$project_root/browser-extension/popup.js" "$stage_dir/popup.js"
mkdir -p "$output_dir"
xcrun safari-web-extension-converter "$stage_dir" --project-location "$output_dir" --app-name "Bridge Safari Connector" --bundle-identifier "dev.bridge.deck.safari" --swift --no-open
cp "$project_root/safari-extension/SafariWebExtensionHandler.swift" "$output_dir/Bridge Safari Connector Extension/SafariWebExtensionHandler.swift"
echo "Safari Web Extension project generated at $output_dir"
