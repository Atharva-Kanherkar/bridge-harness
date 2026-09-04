#!/bin/sh
# Ensure the Claude Agent SDK sidecar has a local node_modules tree so the
# bundled copy at Contents/Resources/sidecar/claude-agent/ can `import
# "@anthropic-ai/claude-agent-sdk"` without a source checkout.
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
src="$project_root/sidecar/claude-agent"

if [ ! -f "$src/index.mjs" ]; then
  echo "prepare-claude-sidecar: missing $src/index.mjs" >&2
  exit 1
fi

if [ ! -f "$src/node_modules/@anthropic-ai/claude-agent-sdk/package.json" ]; then
  if command -v npm >/dev/null 2>&1; then
    npm ci --prefix "$src" --ignore-scripts
  else
    echo "prepare-claude-sidecar: install sidecar deps (npm ci --prefix sidecar/claude-agent --ignore-scripts)" >&2
    exit 1
  fi
fi

echo "Prepared Claude sidecar at $src"
