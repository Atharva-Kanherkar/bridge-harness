#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
profile=${1:-release}
target=$(rustc -vV | sed -n 's/^host: //p')
output_dir="$project_root/src-tauri/binaries"
sidecar="$output_dir/bridge-browser-host-$target"

mkdir -p "$output_dir"
if [ ! -e "$sidecar" ]; then
  : > "$sidecar"
  chmod 700 "$sidecar"
fi

if [ "$profile" = "release" ]; then
  cargo build --manifest-path "$project_root/src-tauri/Cargo.toml" --release --bin bridge-browser-host
  built="$project_root/src-tauri/target/release/bridge-browser-host"
else
  cargo build --manifest-path "$project_root/src-tauri/Cargo.toml" --bin bridge-browser-host
  built="$project_root/src-tauri/target/debug/bridge-browser-host"
fi

cp "$built" "$sidecar"
chmod 700 "$sidecar"
echo "Prepared native messaging host at $sidecar"
