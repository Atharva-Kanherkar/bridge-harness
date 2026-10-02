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

if ! command -v npm >/dev/null 2>&1; then
  echo "prepare-claude-sidecar: npm is required to materialize sidecar node_modules" >&2
  exit 1
fi

for file in index.mjs briefing.mjs input.mjs options.mjs read-only.mjs usage.mjs package.json package-lock.json; do
  if [ ! -f "$src/$file" ]; then
    echo "prepare-claude-sidecar: missing $src/$file" >&2
    exit 1
  fi
done

sdk_dir="$dest/node_modules/@anthropic-ai/claude-agent-sdk"
platform=$(node -p 'process.platform + "-" + process.arch')
# Compare before copying: copying the new lock over the old one first silently
# reused an older SDK tree even when dependencies changed. Also reject staging
# from another architecture, which may contain incompatible optional binaries.
if [ -f "$sdk_dir/package.json" ] && [ ! -L "$sdk_dir" ] \
  && cmp -s "$src/package.json" "$dest/package.json" \
  && cmp -s "$src/package-lock.json" "$dest/package-lock.json" \
  && [ "$(cat "$dest/.bridge-stage-platform" 2>/dev/null || true)" = "$platform" ] \
  && npm ls --prefix "$dest" --omit=dev --all >/dev/null 2>&1; then
  for file in index.mjs briefing.mjs input.mjs options.mjs read-only.mjs usage.mjs; do
    cp "$src/$file" "$dest/$file"
  done
  echo "Prepared Claude sidecar at $dest (reused matching locked dependencies)"
  exit 0
fi

mkdir -p "$(dirname -- "$dest")"
stage=$(mktemp -d "$(dirname -- "$dest")/.claude-agent.XXXXXX")
trap 'rm -rf "$stage"' EXIT HUP INT TERM
for file in index.mjs briefing.mjs input.mjs options.mjs read-only.mjs usage.mjs package.json package-lock.json; do
  cp "$src/$file" "$stage/$file"
done
npm ci --prefix "$stage" --ignore-scripts --omit=dev
npm ls --prefix "$stage" --omit=dev --all >/dev/null
staged_sdk="$stage/node_modules/@anthropic-ai/claude-agent-sdk"
if [ ! -f "$staged_sdk/package.json" ] || [ -L "$staged_sdk" ]; then
  echo "prepare-claude-sidecar: expected a real (non-symlink) SDK in the staged bundle" >&2
  exit 1
fi
printf '%s\n' "$platform" > "$stage/.bridge-stage-platform"
# Keep the previous complete tree until installation has succeeded.
rm -rf "$dest"
mv "$stage" "$dest"
echo "Prepared Claude sidecar at $dest"
