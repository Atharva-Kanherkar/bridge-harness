# codex/issue-5-policy-engine — Test Contract

## Functional Behavior

- `src-tauri/bridge-core/src/policy.rs` is a pure deterministic gate with no model/harness call.
- Every worker request produces exactly one typed `RouteDecision`: `ExecuteInParent`, `ResumeWorker`, `SpawnWorker`, `Queue`, `Reject`, or `RequireUserApproval`, with a stable `RouteReason`.
- Default limits are enforced: 2 concurrent workers, 1 writing worker per worktree, 3 workers per orchestrator turn, 1 strong worker per turn, 1 automatic retry, and depth 1.
- Read-only workers may run concurrently. Overlapping writers queue. Disjoint isolated writers receive a child-worktree requirement. Shared/full writers remain single-writer.
- A user-request budget is keyed by the orchestrator turn ID; a different turn starts with independent counters.
- Capability units are deterministic and audited: fast+low=1, standard+medium=3, strong+high=8, with xhigh/fresh/native/retry multipliers from issue #1.
- Compatible warm workers are preferred over fresh spawn when workspace, role, harness, tier, task family, and owned paths match.
- Normalized owned-path/glob overlap detection is conservative, deterministic, rejects traversal patterns, and is reusable by the future worktree coordinator.
- Every runtime worker launch calls the policy engine before adapter start. Non-spawn decisions never start a process.
- Every decision is appended to the session forest with turn ID, decision, reason, request, budget snapshot, and capability units.
- Spawn usage and provider-reported token/context/runtime data are appended to `usage_ledger` with the same turn ID when available.

## Unit Tests

- One test per decision: execute-in-parent, warm resume, spawn, queue concurrency, queue writer conflict, reject depth, reject worker budget, reject strong cap, reject retry ceiling, reject unit budget, require approval.
- `same_inputs_produce_identical_decision_and_reason` — full decision equality across repeated calls.
- `new_turn_resets_request_budget` — ledger-derived counters do not leak across turn IDs.
- `capability_unit_matrix_and_multipliers_are_stable` — exact expected integer/rounded units.
- `path_overlap_handles_glob_file_nested_and_disjoint_sets` — glob/glob, glob/file, nested, normalized separators, disjoint, and invalid traversal.
- `warm_worker_compatibility_uses_complete_key` — any mismatched compatibility dimension prevents resume.
- `usage_report_normalizes_codex_and_claude_shapes` — token/cache/context/runtime fields map into one ledger record.
- `policy_decision_is_persisted_in_active_forest` — reason and turn ID are queryable.

## Integration / Functional Tests

- `launch_worker` cannot reach `adapter_registry.start` without `RouteDecision::SpawnWorker` or `ResumeWorker`.
- Spawn integration creates an active lease and a policy usage-ledger row keyed to the parent turn.
- A second overlapping writer request records `Queue` and starts no adapter.
- A fourth request in the same turn records `Reject`; the first request on a new turn is independently eligible.
- Full Rust suite passes with existing migration, forest, delegation, archive, and review-fix tests.

## Smoke Tests

- `bun run test` passes.
- `bun run check` passes.
- `bun run build` passes.

## E2E Tests

- Parent turn → typed delegation request → policy spawn decision → lease + ledger + forest event.
- Same turn → conflicting writer → queued decision event and no process spawn.
- New parent turn → counters reset and request becomes eligible.

## Manual / cURL Tests

- N/A for cURL — local policy/SQLite behavior only.
- Confirm `rg 'adapter_registry\.start' src-tauri/src/lib.rs` shows worker start only after the policy decision match.
- Query `session_entries` and `usage_ledger` by turn ID and verify decision reason/units are auditable.
