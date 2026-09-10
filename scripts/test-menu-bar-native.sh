#!/bin/sh
# Deterministic native rendering/formatting checks; no provider, Keychain or UI actions.
set -eu
if [ "$(uname -s)" != Darwin ]; then
  echo 'Native Menu Bar checks require macOS; skipped.'
  exit 0
fi
repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test_dir=$(mktemp -d "${TMPDIR:-/tmp}/bridge-menu-bar-tests.XXXXXX")
trap 'rm -rf "$test_dir"' EXIT HUP INT TERM
case "$(uname -m)" in arm64) target_arch=arm64 ;; *) target_arch=x86_64 ;; esac
xcrun swiftc -module-cache-path "$test_dir/module-cache" -swift-version 5 -target "${target_arch}-apple-macosx12.0" \
  "$repo_dir"/src-tauri/bridge-menu-bar/swift/*.swift \
  "$repo_dir/src-tauri/bridge-menu-bar/tests/main.swift" \
  -o "$test_dir/menu-bar-tests"
"$test_dir/menu-bar-tests" "$repo_dir/src-tauri/bridge-protocol/tests/fixtures/menu-bar-presentation.json"
