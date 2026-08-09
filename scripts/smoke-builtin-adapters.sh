#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
manifest="$repo_root/src-tauri/Cargo.toml"

run_smoke() {
  test_name=$1
  if ! result=$(cargo test --manifest-path "$manifest" -p bridge-core --lib \
    "$test_name" -- --ignored --exact --nocapture 2>&1); then
    printf '%s\n' "$result" >&2
    return 1
  fi
  printf '%s\n' "$result"
  case "$result" in
    *"test result: ok. 1 passed; 0 failed;"*) ;;
    *)
      printf 'Expected exactly one passing smoke test for %s\n' "$test_name" >&2
      return 1
      ;;
  esac
}

# Opt-in only: these tests use the vendor runtimes and authentication already
# configured on this machine. OpenCode also requires BRIDGE_OPENCODE_LIVE_BINARY.
run_smoke claude_adapter::tests::live_stream_json_emits_a_structured_turn
run_smoke claude_adapter::tests::live_claude_session_survives_process_restart
run_smoke codex_adapter::tests::live_app_server_emits_a_structured_turn
run_smoke codex_adapter::tests::live_codex_thread_survives_process_restart
run_smoke opencode_adapter::tests::live_discovery_reads_opencode_go_from_the_structured_provider_api
run_smoke opencode_adapter::tests::live_chat_streams_an_opencode_go_reply
