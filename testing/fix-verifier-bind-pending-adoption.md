# fix/verifier-bind-pending-adoption — Test Contract

Closes the hole where a verification worker cannot bind to an implementation
result that left its change set uncommitted in the worker's own worktree.

## Root cause (established before writing code)

`completion::create_from_worker_result` opens a completion gate only for
`role == "implementation" && status == Completed`:

```rust
if context.role != "implementation" || result.status != WorkerResultStatus::Completed {
    return Ok(None);
}
```

An implementation worker that returns `status: needs_delegation` therefore
records **no `eval_attempts` row at all**. Two things then break:

1. `live_turn::launch_worker_outcome` binds a verification worker by reading
   `SELECT repository_path FROM eval_attempts WHERE session_id=?1 AND status IN
   ('verifying','changes_requested','failed')`. With no row, `query_row` returns
   `QueryReturnedNoRows`, surfaced verbatim as
   `Could not bind verifier to the implementation revision: Database: Query
   returned no rows`.
2. Even if binding were forced through, `settle_verification_result` needs
   `latest_summary` and would fail with `verification worker completed without an
   active completion gate` → the `gate-error` blocked check seen in the report.

The gate itself never required commits — `repository_stamp` records `head` +
`dirtyHash`, so a dirty, commit-less worktree is a perfectly bindable revision.
The blocking condition is the **status**, not the absence of commits.

## Functional Behavior

1. An implementation worker returning `needs_delegation` **with a change set**
   (`files_changed` non-empty after Git reconciliation) opens a completion gate
   exactly like a `completed` result does: verdict `verifying`, an
   `eval_attempts` row whose `repository_path` is the worker's worktree, base
   revision / branch / worker session bound from `worker_adoption::binding`.
2. An implementation worker returning `needs_delegation` with **no** change set
   still opens no gate — there is nothing to verify.
3. Every other non-`completed` status (`failed`, `blocked`, `cancelled`,
   `protocol_invalid`) keeps its current behavior: no gate.
4. A verification worker launched against a task that has a gate binds to that
   gate's `repository_path` (unchanged behavior).
5. A verification worker launched against a task with **no** gate fails with a
   typed, actionable reason instead of a raw rusqlite error. The reason names the
   condition (`implementation_revision_unavailable`) and states the remediation
   (commit/adopt the worker's worktree changes, or re-run the implementation).
6. A genuine database error while resolving the target is still reported as a
   database error — it must not be flattened into "no revision recorded".

## Unit Tests

`src-tauri/bridge-core/src/completion.rs`:
- `needs_delegation_with_changes_opens_a_bindable_gate` — an implementation
  worker whose result status is `needs_delegation` and whose `files_changed` is
  non-empty produces a gate with verdict `verifying`, and
  `verification_target_path` returns that attempt's `repository_path`.
- `needs_delegation_without_changes_opens_no_gate` — same worker, empty
  `files_changed`, returns `None` and leaves `eval_attempts` empty.
- `failed_implementation_opens_no_gate` — `status: failed` with a change set
  still opens no gate (guards against widening the condition too far).
- `verification_target_path_is_none_without_a_gate` — no `eval_attempts` row →
  `Ok(None)`, and `verification_target_missing_reason()` names both the typed
  reason id and the remediation.

`src-tauri/bridge-core/src/live_turn.rs`: the bind site is inside
`launch_worker_outcome`, which needs a live `BridgeCore` (adapters, threads), so
the branch logic is extracted into `completion::verification_target_path` and
covered there. The live_turn change is a three-arm `match` over that helper.

## Integration / Functional Tests

- Existing `verifiers_close_semantic_checks_but_never_shell_checks` must stay
  green: the verification settle path is unchanged.
- Existing `worker_result_opens_gate_and_rebinds_on_new_revision` must stay
  green: the `completed` path is unchanged.
- `cargo test -p bridge-core` in full — the completion gate is read by
  `check_runner`, `learning_job`, `learning_router`, `work`, and `api`, and none
  of those should change behavior for `completed` results.

## Smoke Tests

- `bun run check` (tsc -b + cargo check) is clean.
- `bun run test` (vitest + cargo test) is green.

## E2E Tests

N/A — reproducing the original failure end to end needs a live orchestrator, two
model workers, and a real isolated worktree. The two failure points are covered
by unit tests against the same SQLite schema the runtime uses.

## Manual / cURL Tests

N/A — no HTTP surface. Manual reproduction, for a reviewer with the desktop app:

1. Delegate an implementation task in `isolated` write mode.
2. Have the worker return `status: needs_delegation` with edits left uncommitted.
3. Confirm the routing notice now reports a `completion` gate with verdict
   `verifying` (previously `completion: null`).
4. Delegate the follow-up verification worker; it starts in the implementation
   worktree instead of failing with `Query returned no rows`.
