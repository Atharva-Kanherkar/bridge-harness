# codex/issue-4-typed-delegation — Test Contract

## Functional Behavior

- `delegation.rs` exposes schema-versioned typed delegation requests and worker results as the only internal delegation representation.
- Requests include role, objective, acceptance criteria, known facts, decisions, relevant files, owned paths, write mode, capability tier, effort, verification commands, and output contract; temporary provider transport hints remain typed and non-authoritative until #6.
- Worker results include status (`completed`, `failed`, `cancelled`, `blocked`, `needs_delegation`), summary, changed files, executed tests, decisions, risks, remaining work, suggested next action, and conditional follow-up delegation fields.
- `cancelled` is terminal and never considered retryable; `needs_delegation` requires a suggested role and task and routes back through the parent instead of spawning recursively.
- Fenced `bridge-delegate` remains a transport boundary; both the new schema and legacy `task`/`context` blocks parse immediately into the typed request.
- Malformed structured worker output gets exactly one repair turn in the same worker session. A second failure is forwarded with an explicit `unstructured` label and records a policy/audit event; Bridge never invents a structured result.
- Default delegation depth is one: orchestrator → worker. A worker cannot directly spawn another worker.
- `worker_briefing` and `protocol` require the typed request/result schemas and flat topology.
- Machine blocks remain stripped from user-facing prose without removing unrelated fences.

## Unit Tests

- `typed_request_round_trips_all_fields_and_schema_version` — serialize/parse preserves the complete request.
- `typed_worker_result_round_trips_every_status` — all five status variants round-trip; unknown status is rejected.
- `cancelled_is_terminal_and_not_retryable` — cancellation never enters retry/delegation behavior.
- `needs_delegation_requires_suggestion` — missing suggested role/task fails validation.
- `legacy_fenced_directive_converts_at_parse_boundary` — old `task`/`context` becomes typed objective/known facts and no legacy object escapes.
- `malformed_output_requests_one_same_session_repair_then_unstructured_fallback` — first failure requests repair, second forwards original raw output labeled unstructured.
- `valid_repair_clears_repair_state` — a corrected result after repair is accepted normally.
- `strip_directives_removes_typed_and_legacy_machine_blocks_only` — prose and unrelated code fences remain.
- `flat_protocol_forbids_worker_delegation` — depth one workers are instructed to return `needs_delegation` and depth-two launch is unreachable.
- `worker_briefing_contains_typed_output_contract` — spawned workers receive their objective/criteria/context and exact result schema.

## Integration / Functional Tests

- `lib.rs` launch paths consume only `DelegationRequest`; no `task` or `context` field crosses the parser boundary.
- Parent notification serializes a validated typed worker result; fallback notification explicitly preserves raw text as unstructured.
- Repair requests are sent through the existing runtime for the same child session and are not implemented by spawning a formatter worker.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace delegation` and the full Rust suite pass.

## Smoke Tests

- `bun run test` passes.
- `bun run check` passes.
- `bun run build` passes.

## E2E Tests

- In a runtime-state integration test: malformed child result → same child receives repair prompt → corrected typed result reaches parent.
- In a fallback integration test: malformed child result → malformed repair → parent receives raw output labeled `unstructured` and the audit event is stored.

## Manual / cURL Tests

- N/A for cURL — delegation uses local structured harness processes.
- Inspect the orchestrator/worker briefing strings and confirm workers cannot directly delegate.
- Confirm `rg 'MAX_DEPTH\s*:\s*i64\s*=\s*3|\.task|\.context' src-tauri/bridge-core/src/delegation.rs src-tauri/src/lib.rs` finds no reachable legacy internal fields or depth-three default.
