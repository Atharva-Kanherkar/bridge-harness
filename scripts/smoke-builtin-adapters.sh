#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
manifest="$repo_root/src-tauri/Cargo.toml"

run_smoke() {
  cargo test --manifest-path "$manifest" -p bridge-core "$1" -- \
    --ignored --nocapture
}

# Opt-in only: these tests use the vendor runtimes and authentication already
# configured on this machine. OpenCode also requires BRIDGE_OPENCODE_LIVE_BINARY.
run_smoke live_stream_json_emits_a_structured_turn
run_smoke live_claude_session_survives_process_restart
run_smoke live_app_server_emits_a_structured_turn
run_smoke live_codex_thread_survives_process_restart
run_smoke live_discovery_reads_opencode_go_from_the_structured_provider_api
run_smoke live_chat_streams_an_opencode_go_reply
