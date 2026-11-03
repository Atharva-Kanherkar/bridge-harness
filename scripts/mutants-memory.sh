#!/usr/bin/env bash
# Mutation testing for the account-memory code (testing/feat-memory-hardening.md).
#
# Mutates only the lines this branch changed in the memory modules, the memory
# tools and their wiring, and runs the tests that own them. A surviving mutant
# is a line no test notices changing: kill it with a test, or say in the PR
# why it is equivalent.
#
# In place, because bridge-core include_str!s files outside src-tauri that a
# copied tree lacks. Run it from a scratch worktree, never the one you are
# editing, with a target dir of its own:
#
#   git worktree add --detach ../mutants HEAD
#   CARGO_TARGET_DIR=~/.cache/mutants-target ../mutants/scripts/mutants-memory.sh [base-ref]
#
# base-ref defaults to origin/main.
set -euo pipefail

base="${1:-origin/main}"
root="$(git rev-parse --show-toplevel)"
diff="$(mktemp)"
trap 'rm -f "$diff"' EXIT

git -C "$root" diff "$base" --relative=src-tauri -- \
  'src-tauri/bridge-core/src/memory_*.rs' \
  src-tauri/bridge-core/src/notes/tool.rs \
  src-tauri/bridge-core/src/mcp_apps/server.rs \
  src-tauri/bridge-core/src/opencode_adapter.rs >"$diff"

cd "$root/src-tauri"
exec cargo mutants --package bridge-core --in-place --in-diff "$diff" \
  --exclude-re 'memory_evals|memory_properties' \
  -- --lib -- memory_ notes::tool mcp_apps opencode_adapter
