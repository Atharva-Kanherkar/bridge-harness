#!/usr/bin/env bash
# Mutation testing for the model-switch handoff
# (testing/fix-model-switch-handoff-followups.md).
#
# Mutates only the lines this branch changed in the handoff modules (tool
# facts, the Bridge facts block, the tail, the returned-thread delta,
# chats_history, the account carry and its switch path) and runs the tests that
# own them. A surviving mutant is a line no test notices changing: kill it with
# a test, or say in the PR why it is equivalent.
#
#   scripts/mutants-handoff.sh [base-ref]        # default origin/main
#   MUTANTS_JOBS=4 scripts/mutants-handoff.sh    # parallel copies (default 4)
#   MUTANTS_LIST=1 scripts/mutants-handoff.sh    # list the mutants, run nothing
#   MUTANTS_OUT=<dir> scripts/mutants-handoff.sh # where mutants.out goes (default: outside the repo, in $TMPDIR)
#
# cargo-mutants copies src-tauri into $TMPDIR, but bridge-core include_str!s
# files above src-tauri (testing/, runtimes/, src/, ...). The copies are made
# inside a shim directory that links every other top-level entry of the repo,
# so those paths resolve in each copy exactly as they do in the checkout.
#
# The eval and property modules are tests, not code under test, and are not
# mutated.
set -euo pipefail

base="${1:-origin/main}"
root="$(git rev-parse --show-toplevel)"
diff="$(mktemp)"
shim="$(mktemp -d)"
trap 'rm -f "$diff"; rm -rf "$shim"' EXIT

git -C "$root" diff "$base" --relative=src-tauri -- \
  src-tauri/bridge-core/src/tool_facts.rs \
  src-tauri/bridge-core/src/handoff_facts.rs \
  src-tauri/bridge-core/src/restoration.rs \
  src-tauri/bridge-core/src/provider_threads.rs \
  src-tauri/bridge-core/src/history_tools.rs \
  src-tauri/bridge-core/src/thread_carry.rs \
  src-tauri/bridge-core/src/sessions.rs >"$diff"

for entry in "$root"/* "$root"/.[!.]*; do
  name="$(basename "$entry")"
  case "$name" in src-tauri | .git | node_modules) continue ;; esac
  ln -s "$entry" "$shim/$name"
done

cd "$root/src-tauri"
list=()
if [ -n "${MUTANTS_LIST:-}" ]; then list=(--list); fi
out="${MUTANTS_OUT:-${TMPDIR:-/tmp}/mutants-handoff}"
mkdir -p "$out"
TMPDIR="$shim" exec cargo mutants --package bridge-core --jobs "${MUTANTS_JOBS:-4}" --output "$out" ${list[@]+"${list[@]}"} \
  --in-diff "$diff" \
  --exclude-re 'handoff_eval|handoff_properties' \
  -- --lib -- tool_facts handoff_facts restoration provider_threads history_tools thread_carry sessions::
