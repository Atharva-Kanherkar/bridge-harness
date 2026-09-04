#!/bin/sh
# Stage a production Claude sidecar tree for Tauri to copy into
# Contents/Resources/sidecar/claude-agent/.
#
# Bun workspaces hoist @anthropic-ai/claude-agent-sdk into a symlink under
# node_modules/.bun/. Tauri's resource walker does not follow those links, so
# bundling sidecar/claude-agent/ in place ships the JS with no SDK. npm ci in
# an isolated staging dir writes a real tree the bundler can copy.
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
src="$project_root/sidecar/claude-agent"
dest="$project_root/src-tauri/resources/sidecar/claude-agent"

if [ ! -f "$src/index.mjs" ]; then
  echo "prepare-claude-sidecar: missing $src/index.mjs" >&2
  exit 1
fi

if ! command -v npm >/dev/null 2>&1; then
  echo "prepare-claude-sidecar: npm is required to materialize sidecar node_modules" >&2
  exit 1
fi

rm -rf "$dest"
mkdir -p "$dest"

for file in index.mjs briefing.mjs input.mjs options.mjs package.json package-lock.json; do
  if [ ! -f "$src/$file" ]; then
    echo "prepare-claude-sidecar: missing $src/$file" >&2
    exit 1
  fi
  cp "$src/$file" "$dest/$file"
done

npm ci --prefix "$dest" --ignore-scripts

sdk_dir="$dest/node_modules/@anthropic-ai/claude-agent-sdk"
if [ ! -f "$sdk_dir/package.json" ] || [ -L "$sdk_dir" ]; then
  echo "prepare-claude-sidecar: expected a real (non-symlink) SDK at $sdk_dir" >&2
  exit 1
fi

echo "Prepared Claude sidecar at $dest"
