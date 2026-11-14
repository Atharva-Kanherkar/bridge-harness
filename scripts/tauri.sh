#!/bin/sh
# Stage native helpers before Tauri starts its 180-second dev-server deadline.
# A cold Rust build can take longer than that; it is not a Vite startup failure.
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"

if [ "${1:-}" = "dev" ]; then
  bun run prepare:browser-host:dev
  bun run prepare:daemon:dev
  bun run prepare:voice-helper
fi

# A Linux bundle runs linuxdeploy, whose patchelf corrupts the Claude SDK's
# native binary. The wrapper skips that one file; a PATCHELF the caller chose
# still handles everything else.
if [ "${1:-}" = "build" ] && [ "$(uname -s)" = "Linux" ]; then
  BRIDGE_REAL_PATCHELF=${PATCHELF:-patchelf}
  PATCHELF="$project_root/scripts/linuxdeploy-patchelf.sh"
  export BRIDGE_REAL_PATCHELF PATCHELF
fi

# Use the installed CLI directly so this does not recurse through the package
# script. exec keeps Tauri in charge of Vite and desktop process cleanup.
exec "$project_root/node_modules/.bin/tauri" "$@"
