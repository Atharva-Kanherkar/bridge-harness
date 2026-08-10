# codex/issue-167-lifecycle-states — Test Contract

## Functional Behavior

- This PR adds only the agent lifecycle state machine and its coordinator required by
  #167. It does not add RPC (#169), UI (#170), real payload recipes (#172), an auth
  service, a credential store, an OAuth client, a logout path, or any credential
  deletion. Vendor authentication and configuration remain untouched and vendor-owned.
- Ten states exist: `not_installed`, `installing`, `installed`, `ready`, `running`,
  `stopping`, `uninstalling`, `external`, `broken`, `repairable`. Each has a stable
  snake_case wire string that round-trips through `FromStr`/`Display`.
- The transition relation is closed and explicit. Every ordered pair of states is
  either in the locked matrix or rejected with a typed error naming both states.
  Nothing infers a transition from state adjacency.
- `installed` means Bridge owns a receipt-bound payload. `ready` means the existing
  integration's own non-destructive readiness check passed. `running` means a live
  process exists. These are three independent observations, never collapsed into one
  boolean.
- `external` means a user-managed runtime is discoverable. An external agent can
  never enter `uninstalling`: Bridge has no receipt for it and must not remove it.
  The rejection is a distinct typed refusal, not a generic invalid transition.
- Uninstall cannot race a live process. `running` has no edge to `uninstalling`; the
  only path is `running → stopping → uninstalling`, so the process is proven stopped
  before any payload removal, and a launch cannot be handed a path that is being
  deleted.
- Spawn-on-use launches only from `ready`. A launch attempted from any other state is
  refused without side effects.
- A vendor reporting a missing login or API key is not a Bridge failure. Readiness
  returns a vendor-blocked outcome, the agent stays `installed` rather than becoming
  `broken` or `ready`, the vendor's own message is preserved verbatim, and it stays
  identifiable as a vendor error rather than being recast as Bridge credential state.
- Crash loops are bounded. Consecutive launch or run failures are counted against a
  budget; once exhausted the agent stays `broken` and further retries are refused
  until the budget is explicitly reset by a successful run or an operator retry.
- Failure context is retained but redacted through `secret_interception::sanitize`
  and bounded in length, so a stderr tail carrying a token never lands in lifecycle
  state.
- Payload conditions map deterministically to states: a `Repairable` payload resolves
  to `repairable`, a missing payload with no external runtime resolves to
  `not_installed`, and an installed payload whose entrypoint is not executable
  resolves to `repairable` rather than `ready`.
- App shutdown stops every running agent and leaves no agent in `running` or
  `stopping`. Idle shutdown is opt-in: with no idle timeout configured, a
  long-untouched running agent is left alone.
- The state machine and coordinator are host-agnostic. Readiness probing and process
  launching are injected, so no test spawns a real vendor process.

## Unit Tests

- `every_state_pair_matches_the_locked_transition_matrix` — all 100 ordered pairs
  agree with `LEGAL_TRANSITIONS`; the matrix has no duplicates.
- `wire_strings_round_trip_and_reject_unknown_states` — `as_str`/`FromStr` round-trip
  for all ten states; an unknown string is a typed parse error.
- `running_cannot_reach_uninstalling_without_stopping` — the direct edge is rejected,
  and `running → stopping → uninstalling` is accepted.
- `external_runtimes_cannot_be_uninstalled` — an external agent's uninstall request
  is a typed refusal naming external ownership, and the payload store is never asked
  to remove anything.
- `settled_state_separates_payload_readiness_and_process` — the eight relevant
  observation combinations each resolve to exactly one expected state, including an
  installed-but-unexecutable payload resolving to `repairable`.
- `vendor_auth_block_keeps_the_agent_installed_and_the_message_intact` — a
  vendor-blocked readiness outcome resolves to `installed`, preserves the vendor
  string verbatim, and reports as vendor-owned rather than a Bridge failure.
- `failure_budget_bounds_crash_loops_and_redacts_context` — consecutive failures
  exhaust the budget, further retries are refused, a successful run resets it, and a
  recorded failure containing a credential-shaped value is stored redacted and
  truncated.
- `spawn_on_use_launches_only_from_ready` — launching from `ready` succeeds and moves
  to `running`; launching from each other state is refused with no launch attempted.
- `uninstall_stops_a_running_process_before_removing_the_payload` — the recorded call
  order proves the process stopped before the payload was removed.
- `app_shutdown_stops_every_running_agent` — no agent remains `running` or
  `stopping`, and each stop is attributed to `ShutdownReason::AppShutdown`.
- `idle_shutdown_is_opt_in_and_bounded` — with no timeout configured nothing is
  stopped; with a timeout, only agents idle beyond it are stopped.
- `cancelling_an_install_returns_to_not_installed` — an install cancelled mid-flight
  leaves no active receipt and the state returns to `not_installed`.
- `repair_and_retry_paths_are_explicit` — `repairable → installing → installed` and
  `broken → installing` are accepted; `broken → ready` is rejected.

## Integration / Functional Tests

- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core agent_lifecycle`
  passes with injected fakes only — no network, no vendor process, no real payload.
- The coordinator drives a real `ManagedPayloadStore` over a temporary managed root
  for install and uninstall, proving the lifecycle and the #165 engine agree on
  ownership: after uninstall there is no active receipt, and an external candidate
  beside it is untouched.
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core` passes.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` passes.
- `scripts/check-builtin-adapters.sh` remains green, proving #162's Claude, Codex,
  and OpenCode compatibility report and normalization snapshots did not drift.
- `bun run build` passes.
- `bun run test` passes, including the Claude sidecar, frontend, and Rust workspace.

## Smoke Tests

- Drive one agent through `not_installed → installing → installed → ready → running
  → stopping → ready → uninstalling → not_installed` against a temporary managed
  root and confirm each transition is accepted and the final state has no receipt.
- Point the coordinator at an external candidate with no managed payload, confirm
  `external`, and confirm an uninstall attempt is refused with the payload untouched.
- Exhaust a crash-loop budget and confirm the agent settles in `broken` with redacted
  failure context and refuses further retries.

## E2E Tests

N/A for desktop E2E in this engine-only PR. The RPC surface lands in #169 and the
desktop Plug / Play / Remove flow in #170; there is no user-reachable path yet.

## Manual / cURL Tests

- N/A for cURL: this PR adds no RPC or HTTP surface.
- Manually inspect a recorded failure context and confirm no token, key, home path,
  or vendor credential appears in it.
- Manually confirm that no state, error, or recorded field represents vendor
  authentication as Bridge-owned credential state.

## Deliberately Out Of Scope

- Wiring the coordinator into the real Claude, Codex, and OpenCode adapters. Their
  readiness checks are reused through an injected probe; pointing them at managed
  payloads is #172.
- Persisting lifecycle state. Transitions are validated in memory; callers persist an
  accepted transition before publishing events, matching `worker_lifecycle`.
