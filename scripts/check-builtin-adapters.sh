#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
manifest="$repo_root/src-tauri/Cargo.toml"
output=${1:-"$repo_root/src-tauri/target/builtin-adapter-compatibility.json"}

case "$output" in
  -*) output="$repo_root/$output" ;;
esac
if [ -d "$output" ]; then
  printf 'Report output is a directory: %s\n' "$output" >&2
  exit 2
fi

run_exact() {
  test_name=$1
  if ! result=$(cargo test --manifest-path "$manifest" -p bridge-core --lib \
    "$test_name" -- --exact 2>&1); then
    printf '%s\n' "$result" >&2
    return 1
  fi
  printf '%s\n' "$result"
  case "$result" in
    *"test result: ok. 1 passed; 0 failed;"*) ;;
    *)
      printf 'Expected exactly one passing test for %s\n' "$test_name" >&2
      return 1
      ;;
  esac
}

mkdir -p "$(dirname -- "$output")"
temporary=$(mktemp "${output}.tmp.XXXXXX")
trap 'rm -f "$temporary"' EXIT HUP INT TERM

prefix=builtin_compatibility::builtin_compatibility_tests
run_exact "$prefix::built_in_contract_has_exactly_the_existing_agents"
run_exact "$prefix::built_in_contract_preserves_transport_runtime_and_vendor_auth_boundaries"
run_exact "$prefix::built_in_contract_matches_adapter_descriptors"
run_exact "$prefix::compatibility_report_is_deterministic_and_schema_versioned"
run_exact "$prefix::representative_provider_streams_match_normalized_snapshots"

# Briefing standing is certified separately from the compatibility report above,
# by the shared adversarial suite. An adapter cannot advertise support without it.
briefing_prefix=briefing_conformance::tests
run_exact "$briefing_prefix::the_conformance_suite_gates_the_capability"
run_exact "$briefing_prefix::the_suite_runs_every_required_fixture_in_order"
run_exact "$briefing_prefix::an_adapter_that_may_not_brief_fails_the_suite_rather_than_skipping_it"

cargo run --quiet --manifest-path "$manifest" -p bridge-core \
  --example builtin-compatibility-report >"$temporary"
python3 -m json.tool "$temporary" >/dev/null
mv "$temporary" "$output"
trap - EXIT HUP INT TERM

printf 'Built-in adapter compatibility report: %s\n' "$output"
