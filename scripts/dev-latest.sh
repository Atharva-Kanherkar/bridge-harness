#!/bin/sh
# One-command dev loop: pull latest main, install deps if the lockfile
# changed, and launch the Tauri app in dev mode. Refuses to run on a dirty
# working tree instead of stashing, so local work never gets silently
# clobbered by the pull.
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"

branch=$(git rev-parse --abbrev-ref HEAD)
if [ "$branch" != "main" ]; then
  printf 'Refusing to auto-pull: current branch is "%s", not "main".\n' "$branch" >&2
  printf 'Switch to main first, or run `bun run tauri dev` directly on this branch.\n' >&2
  exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
  printf 'Refusing to auto-pull: working tree has uncommitted changes.\n' >&2
  printf 'Commit, stash, or discard them, then re-run.\n' >&2
  exit 1
fi

lockfile_before=$(git rev-parse HEAD:bun.lock 2>/dev/null || true)
git pull --ff-only
lockfile_after=$(git rev-parse HEAD:bun.lock 2>/dev/null || true)

if [ "$lockfile_before" != "$lockfile_after" ]; then
  echo "bun.lock changed, running bun install"
  bun install
fi

exec bun run tauri dev
