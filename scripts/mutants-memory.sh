#!/usr/bin/env bash
# Mutation testing for the account-memory code (testing/feat-memory-hardening.md).
#
# Mutates only the lines this branch changed in the memory modules, the memory
# tools and their wiring, and runs the tests that own them. A surviving mutant
# is a line no test notices changing: kill it with a test, or say in the PR
# why it is equivalent.
#
#   scripts/mutants-memory.sh [base-ref]        # default origin/main
#   MUTANTS_JOBS=4 scripts/mutants-memory.sh    # parallel copies (default 4)
#
# cargo-mutants copies src-tauri into $TMPDIR, but bridge-core include_str!s
# files above src-tauri (testing/, runtimes/, src/, ...). The copies are made
# inside a shim directory that links every other top-level entry of the repo,
# so those paths resolve in each copy exactly as they do in the checkout.
#
# The maintenance loop and the live provider turn are excluded: they only run
# against a real host and provider, and the evals cover them.
set -euo pipefail

base="${1:-origin/main}"
root="$(git rev-parse --show-toplevel)"
diff="$(mktemp)"
shim="$(mktemp -d)"
trap 'rm -f "$diff"; rm -rf "$shim"' EXIT

git -C "$root" diff "$base" --relative=src-tauri -- \
  'src-tauri/bridge-core/src/memory_*.rs' \
  src-tauri/bridge-core/src/notes/tool.rs \
  src-tauri/bridge-core/src/mcp_apps/server.rs \
  src-tauri/bridge-core/src/opencode_adapter.rs >"$diff"

for entry in "$root"/* "$root"/.[!.]*; do
  name="$(basename "$entry")"
  case "$name" in src-tauri | .git | node_modules) continue ;; esac
  ln -s "$entry" "$shim/$name"
done

cd "$root/src-tauri"
TMPDIR="$shim" exec cargo mutants --package bridge-core --jobs "${MUTANTS_JOBS:-4}" \
  --in-diff "$diff" \
  --exclude-re 'memory_evals|memory_properties|start_consolidation_maintenance|one_bounded_turn' \
  -- --lib -- memory_ notes::tool mcp_apps opencode_adapter
