#!/bin/sh
# Materialize the headless xterm helper without symlinks for the app bundle.
set -eu
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dest="$project_root/src-tauri/resources/sidecar/terminal-state"
mkdir -p "$dest"
cp "$project_root/sidecar/terminal-state/index.mjs" "$dest/index.mjs"
cp "$project_root/sidecar/terminal-state/state.mjs" "$dest/state.mjs"
cp "$project_root/sidecar/terminal-state/package.json" "$dest/package.json"
cp "$project_root/sidecar/terminal-state/package-lock.json" "$dest/package-lock.json"
cp "$project_root/THIRD_PARTY_NOTICES.md" "$dest/THIRD_PARTY_NOTICES.md"
mkdir -p "$dest/orca"
cp "$project_root/sidecar/terminal-state/orca/"*.mjs "$dest/orca/"
npm ci --prefix "$dest" --ignore-scripts --omit=dev
