# codex/issue-162-agent-regression — Test Contract

## Functional Behavior

- Bridge exposes exactly the existing built-in agent identities `claude`, `codex`, and `opencode`; this PR does not add, remove, rename, install, or launch an agent.
- A versioned compatibility contract records each built-in's current transport, runtime source, credential owner, native-resume behavior, capability set, sandbox support, and stable model identifiers.
- Claude remains bound to the Bridge Node sidecar over `@anthropic-ai/claude-agent-sdk`; Codex remains bound to `codex app-server --listen stdio://`; OpenCode remains bound to `opencode serve` over its authenticated loopback HTTP API.
- Vendor account authentication and configuration remain vendor-owned. Bridge may generate an ephemeral OpenCode loopback-server password, but it does not own, migrate, store, or delete the user's vendor credentials.
- Representative provider event streams for all three built-ins normalize to stable Bridge events for session start, assistant and reasoning streaming, final messages, plans, file changes, tool activity, approvals, errors, completion, and usage where the provider exposes them. These fixtures protect the exercised wire shapes; they are not proof of every advertised capability.
- The compatibility report is deterministic and independent of whether provider binaries or credentials are installed on the machine running it.
- Existing adapter availability discovery, model routing, session persistence, process lifecycle, and runtime behavior remain unchanged.

## Unit Tests

- `built_in_contract_has_exactly_the_existing_agents` — the static contract contains only `claude`, `codex`, and `opencode`, with unique stable identities.
- `built_in_contract_preserves_transport_runtime_and_vendor_auth_boundaries` — each entry pins its current transport/runtime source and `vendor` credential ownership.
- `built_in_contract_matches_adapter_descriptors` — stable capabilities, sandbox modes, static model identifiers, and static default models agree with the actual adapter descriptors. Native-resume policy is locked by the golden report because live support probes depend on locally installed runtimes.
- `compatibility_report_is_deterministic_and_schema_versioned` — serialization matches the checked-in v1 golden JSON byte-for-byte, including labels, transports, runtime sources, native-resume policy, serde field names, capabilities, sandboxes, and models.
- `representative_provider_streams_match_normalized_snapshots` — fixture event sequences for Claude, Codex, and OpenCode yield the expected normalized event summaries.
- Existing adapter-module tests for structured turn frames, interruption, permissions, resume parameters, executable precedence, failure context, and write-mode enforcement continue to pass.

## Integration / Functional Tests

- `cargo test -p bridge-core builtin_compatibility` passes without provider binaries or credentials.
- `cargo test -p bridge-core` passes.
- `bun run build` passes.
- `bun run test` passes, including the Claude sidecar tests, frontend tests, and Rust workspace tests.
- Generating the report with the checked-in report command parses the output as JSON before atomically publishing it, and the unit test requires the same output to match the checked-in golden report.

## Smoke Tests

- Existing ignored live tests remain opt-in and skip during normal CI:
  - `claude_adapter::tests::live_stream_json_emits_a_structured_turn`
  - `claude_adapter::tests::live_claude_session_survives_process_restart`
  - `codex_adapter::tests::live_app_server_emits_a_structured_turn`
  - `codex_adapter::tests::live_codex_thread_survives_process_restart`
  - `opencode_adapter::tests::live_discovery_reads_opencode_go_from_the_structured_provider_api`
  - `opencode_adapter::tests::live_chat_streams_an_opencode_go_reply`
- The compatibility report command runs successfully on a machine with none of the three provider runtimes installed.
- The smoke command requires each fully qualified live test name to execute exactly one test, so a renamed or removed test fails instead of reporting a false green.

## E2E Tests

- N/A for automated CI — real end-to-end turns require vendor runtimes and authenticated vendor accounts.
- Manual desktop verification: start one existing session with each locally configured agent and confirm streaming, interruption, and subsequent native resume behave exactly as before this PR.

## Manual / cURL Tests

- Run the compatibility report command documented by this PR twice and confirm byte-identical JSON output.
- Inspect the report and confirm credential ownership is `vendor` for every built-in and no credential value, path, token, or environment-variable value is present.
- Known compatibility wart: OpenCode currently emits `turn.completed` for `session.idle` even when it follows `session.error`. The v1 fixture locks this existing sequence deliberately; correcting the runtime state transition is outside this regression-only PR.
- N/A for cURL — Claude and Codex use local stdio processes; OpenCode's loopback API is launched and authenticated internally by the existing adapter.
