#!/bin/sh
# Build the bridged daemon and stage it as a Tauri external binary, so the
# desktop app ships (and can later spawn/attach to) the daemon it shares its
# data directory with.
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
profile=${1:-release}
target=$(rustc -vV | sed -n 's/^host: //p')
output_dir="$project_root/src-tauri/binaries"
sidecar="$output_dir/bridged-$target"

mkdir -p "$output_dir"
if [ ! -e "$sidecar" ]; then
  : > "$sidecar"
  chmod 700 "$sidecar"
fi

if [ "$profile" = "release" ]; then
  cargo build --manifest-path "$project_root/src-tauri/Cargo.toml" --release -p bridged
  built="$project_root/src-tauri/target/release/bridged"
else
  cargo build --manifest-path "$project_root/src-tauri/Cargo.toml" -p bridged
  built="$project_root/src-tauri/target/debug/bridged"
fi

cp "$built" "$sidecar"
chmod 700 "$sidecar"
echo "Prepared bridged daemon at $sidecar"
