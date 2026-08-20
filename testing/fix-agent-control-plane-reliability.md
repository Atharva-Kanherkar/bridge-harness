# fix/agent-control-plane-reliability — Test Contract

Locked before implementation. Source: issue "Agent control-plane reliability:
steering, composer + action, and retry-amplified worker failures".

The four defects are one control-plane bug: Bridge removes user control exactly
when an agent needs supervision, and turns recoverable model-output mistakes into
repeated paid turns.

---

## Functional Behavior

### A. Durable active-turn input contract

- A new protocol method `sessions/submit_input` accepts `{sessionId, text}` and
  returns `{disposition, queuedInputId?, interceptions[]}`.
- `disposition` is exactly one of `startedNewTurn | steeredActiveTurn |
  queuedForPhaseBoundary`. Any other value is rejected by the contract.
- No active turn → the existing new-turn path runs unchanged → `startedNewTurn`.
- Active root turn, adapter advertises the `steering` capability → the text is
  delivered to the live provider immediately → `steeredActiveTurn`. Bridge never
  calls a second `turn/start` against a provider that cannot take one.
- Active root turn, adapter does not advertise `steering` → the text is written
  to a durable queue and returns `queuedForPhaseBoundary`. It is delivered at the
  next phase boundary (the session's turn completing) exactly once.
- Delivery is CAS-guarded (`WHERE state='queued'`), so a replay, a reconnect, or
  two concurrent drains cannot deliver the same row twice or lose it.
- Queued rows survive a daemon restart: they live in SQLite, not in memory.
- `interrupt` stays a separate method. Submitting input never cancels a turn.
- Secret interception, `@file` context expansion, and slash-command policy run
  through the same boundary for new-turn, steered, and queued input. Session
  control commands (`/clear`, `/compact`, `/usage`, `/new`, `/reset`) are refused
  while a turn is active, with an error naming why — the same policy decision, at
  the same boundary, not a silently different path.
- Worker sessions stay policy-controlled: `submit_input` refuses them exactly as
  `send_turn` does.
- User steering takes priority over queued automatic recovery: when queued user
  input exists at a phase boundary, it is delivered and the automatic
  delegation-correction turn for that boundary is suppressed and recorded as
  `preempted_by_user_input`.

### B. Dock `+` performs its named action

- The dock `ComposerPill`'s `+` opens the workspace-create dialog — what its
  `New workspace` label says.
- It never clears the composer draft.
- It stays usable while the session is working.
- The hero (welcome) `+` keeps opening the same dialog.

### C. Rust owns the coordination schema

- `DelegationRequest` and `WorkerResult` no longer refuse unknown fields; an
  extra explanatory key is ignored, not fatal.
- The host derives, rather than requiring the model to author: `schemaVersion`,
  empty collections, `writeMode`, `capabilityTier`, `effort`, and
  `outputContract` (derived from role).
- Role aliases normalize: `implementer`/`implementation-verifier`/`coder`/
  `reviewer`/`researcher`/`planner`/`docs` → the canonical roles.
- Status aliases normalize: `success`/`done`/`ok` → `completed`, `error` →
  `failed`, `escalate`/`needs-delegation` → `needs_delegation`.
- `suggestedNextAction` aliases normalize, and an unrecognized value falls back
  to a status-derived default instead of failing the envelope.
- Write mode is **clamped by role in Rust**: read-only roles (research,
  verification, planning, documentation) are read-only no matter what the model
  asked for. Relaxing the wire format does not relax authority.
- A formatting/parse failure is recorded as `WorkerResultStatus::ProtocolInvalid`
  — never rewritten into `Failed`. `protocol_invalid` is not retryable and does
  not count as task failure.
- Usable prose survives: the raw text of an unparseable result is preserved as
  the summary/evidence rather than discarded.
- `needs_delegation` no longer requires the worker to invent a closed
  `suggestedRole`; the host derives one. The depth-one prompt states a bounded
  escalation request instead of an absolute prohibition. Depth, fanout, path
  scope, sandbox, concurrency, and spend limits stay in Rust and are unchanged.

### D. Evidence-based, user-visible retries

- The orchestrator correction budget for an invalid delegation request drops from
  3 to 1, and normalization is attempted first (deterministic, no model turn).
- A worker task auto-retries only for a **classified transient** failure
  (timeout / connection reset / rate limit / 5xx / "temporarily"), at most once,
  and only when there is a hot process and the per-objective retry budget allows.
- A permanent failure (a failed test, an explicit permanent cause) never
  auto-retries.
- `protocol_invalid` never auto-retries.
- Terminal worker failure reaches the orchestrator and the user immediately with
  the real cause and an explicit Retry action (`sessions/retry_worker_task`).
- Correction turns, repair turns, and task-retry turns are counted separately in
  telemetry (`recovery.correction`, `recovery.repair`, `recovery.task_retry`).

---

## Unit Tests

### Rust — protocol (`bridge-protocol`)

- `submit_input_params_round_trip_and_reject_unknown_modes` — `SubmitInputParams`
  round-trips; `SubmitInputResult` round-trips; an unknown disposition string
  fails to deserialize.
- Existing coverage tests must stay green: `TYPED_METHODS` 1:1 with
  `MethodName::ALL`, the generated-artifact drift test, and the shell crate's
  `generate_handler!` ↔ registry parity test.

### Rust — core input contract (`bridge-core::live_turn` / `session_input`)

- `submit_input_starts_a_new_turn_when_no_turn_is_active`
- `submit_input_steers_a_steering_capable_adapter_without_starting_a_turn`
- `submit_input_queues_for_a_non_steering_adapter_and_returns_the_row_id`
- `queued_input_is_delivered_exactly_once_under_concurrent_drains`
- `queued_input_survives_reconnect_and_is_not_duplicated_by_replay`
- `submit_input_refuses_worker_sessions`
- `submit_input_refuses_session_control_slash_commands_while_a_turn_is_active`
- `submit_input_intercepts_secrets_on_every_disposition`
- `user_steering_preempts_the_automatic_delegation_correction`

### Rust — delegation normalization (`bridge-core::delegation`)

- `delegation_request_defaults_transport_fields_from_role`
- `delegation_request_normalizes_role_aliases`
- `delegation_request_ignores_extra_explanatory_fields`
- `delegation_request_clamps_write_mode_for_read_only_roles`
- `worker_result_normalizes_status_and_next_action_aliases`
- `worker_result_accepts_needs_delegation_without_suggested_role`
- `unparseable_worker_output_is_protocol_invalid_not_failed`
- `protocol_invalid_result_is_not_retryable`
- `worker_result_preserves_raw_prose_as_evidence`

### Rust — retry policy (`bridge-core::worker_pool` / `retry`)

- `transient_failure_retries_once_and_records_the_changed_condition`
- `permanent_failure_does_not_auto_retry`
- `failed_test_evidence_classifies_permanent`
- `protocol_invalid_never_retries`
- `retry_budget_is_per_objective_and_exhausts`
- `recovery_turn_kinds_are_counted_separately`

### Frontend (Vitest + Testing Library)

- `ComposerPill` — stays editable while `working`; both Stop and the submit
  affordance are reachable.
- `ComposerPill` — the submit affordance is labelled `Steer` when the adapter
  advertises steering and `Queue` when it does not.
- `ComposerPill` — the dock `+` calls its handler and leaves a non-empty draft
  untouched.
- `App` wiring — active-turn submit calls `submitInput`, restores the draft when
  delivery fails, and renders the queued/delivered state.
- Worker view renders the non-messageable notice and no composer.
- Model switching stays disabled while a turn is active.

---

## Integration / Functional Tests

- `bridged` dispatch routes `sessions/submit_input` to the core entry point and
  decodes/encodes the typed params/result (existing dispatch tests cover the
  round trip shape).
- The generated TypeScript contract exposes `sessions/submit_input` with the
  disposition union, so `bun run build` fails if the wire field is renamed.
- A fixture corpus of historical invalid envelopes (invalid `suggestedRole`,
  invalid `suggestedNextAction`, unknown fields, missing fence) is asserted
  before/after: after the change each fixture either parses or is classified
  `protocol_invalid`, and none of them produce an automatic model turn.

## Smoke Tests

- `bun run build` — green.
- `bun run test` — green (sidecar + vitest + `cargo test --workspace`).
- `cargo run -p bridge-protocol --bin generate-protocol-artifacts` produces no
  diff after the change is committed (drift test enforces this).

## E2E Tests

N/A in CI — Bridge is a Tauri desktop app with no automated E2E harness in this
repo. The equivalent checks are the live smoke steps below, which require a
signed-in provider and a GUI session.

## Manual / Live Verification

Recorded honestly: which of these ran, and which could not, is stated in the PR.

1. Codex, Claude, and OpenCode: start a long turn, submit steering input during
   the work, verify the visible acknowledgement (steered vs queued), then verify
   Stop still interrupts.
2. Restart `bridged` with one queued follow-up and confirm exactly-once delivery
   after reconnect.
3. Dock `+` with a non-empty draft: the workspace dialog opens and the draft is
   still there afterwards.
