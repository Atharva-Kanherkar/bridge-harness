#!/bin/sh
# Build the isolated local-dictation helper and stage it as a Tauri external
# binary. The helper dynamically loads only a separately verified runtime;
# compiling it never downloads an engine or model.
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
target=$(rustc -vV | sed -n 's/^host: //p')
output_dir="$project_root/src-tauri/binaries"
helper="$output_dir/bridge-voice-helper-$target"

mkdir -p "$output_dir"
cc -std=c11 -O2 -Wall -Wextra -Werror \
  "$project_root/src-tauri/voice-helper/main.c" \
  -o "$helper"
chmod 700 "$helper"
echo "Prepared local dictation helper at $helper"
