#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
manifest="$repo_root/src-tauri/Cargo.toml"
output=${1:-"$repo_root/target/builtin-adapter-compatibility.json"}

mkdir -p "$(dirname -- "$output")"
temporary=$(mktemp "${output}.tmp.XXXXXX")
trap 'rm -f "$temporary"' EXIT HUP INT TERM

cargo test --manifest-path "$manifest" -p bridge-core builtin_compatibility
cargo run --quiet --manifest-path "$manifest" -p bridge-core \
  --example builtin-compatibility-report >"$temporary"
mv "$temporary" "$output"
trap - EXIT HUP INT TERM

printf 'Built-in adapter compatibility report: %s\n' "$output"
