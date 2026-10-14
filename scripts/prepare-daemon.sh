#!/bin/sh
# Build the bridged daemon and stage it as a Tauri external binary, so the
# desktop app ships (and can later spawn/attach to) the daemon it shares its
# data directory with. The `bridge` CLI is staged beside it as `bridge-cli`:
# the Linux bundle ships it for desktop surfaces (the Omarchy bar widget), and
# `/usr/bin/bridge` already belongs to iproute2's network bridge tool.
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
profile=${1:-release}
target=$(rustc -vV | sed -n 's/^host: //p')
output_dir="$project_root/src-tauri/binaries"
sidecar="$output_dir/bridged-$target"
cli="$output_dir/bridge-cli-$target"
target_dir=${CARGO_TARGET_DIR:-"$project_root/src-tauri/target"}

mkdir -p "$output_dir"
for staged in "$sidecar" "$cli"; do
  if [ ! -e "$staged" ]; then
    : > "$staged"
    chmod 700 "$staged"
  fi
done

if [ "$profile" = "release" ]; then
  cargo build --manifest-path "$project_root/src-tauri/Cargo.toml" --release -p bridged -p bridge-client --bin bridged --bin bridge
  built_dir="$target_dir/release"
else
  cargo build --manifest-path "$project_root/src-tauri/Cargo.toml" -p bridged -p bridge-client --bin bridged --bin bridge
  built_dir="$target_dir/debug"
fi

cp "$built_dir/bridged" "$sidecar"
chmod 700 "$sidecar"
cp "$built_dir/bridge" "$cli"
chmod 755 "$cli"
echo "Prepared bridged daemon at $sidecar and the bridge CLI at $cli"
