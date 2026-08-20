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

---

# Contract amendment — review round 1

Five verified findings against the first implementation. Each is confirmed
against the code below, and each amends or replaces a clause above.

## A1 — an unroutable verifier must not poison the gate (was finding 1)

`fail_reserved_worker` synthesizes a `Failed` **verification** result for the
reserved session. That runs the settle path: `settle_verification_result` finds no
gate, returns `Err`, and `live_turn` falls back to `completion::record_gate_error`,
which supersedes every live attempt for the parent and writes a new attempt with
status `failed` and no `escalation`. `completion_allows_ready` returns `true` for a
failed verdict **only** when `escalation == VERIFY_DEADLINE_ESCALATION`, so the
parent is pinned `waiting` forever, and `check_runner::stalled_attempts` cannot
rescue it because it only considers attempts still in `verifying`/
`changes_requested`.

Replaces contract item 5. The launch must be **aborted**, not settled:
`delete_reserved_worker` removes the never-started session, the parent is told
through `report_worker_launch_failure`, and `reconcile_parent_readiness` runs so
the parent is not left pinned by a reservation that no longer exists. The same
applies to a genuine database error on the same path.

- `an_unroutable_verifier_is_aborted_without_a_gate` — no `eval_attempts` row is
  created, no `worker_runtime`/`worker_leases`/`sessions` row survives for the
  reserved verifier, and the parent is not left `waiting` on it.

## A2 — Git-derived paths always replace the worker's claim (was finding 2)

`worker_adoption::reconcile_with_derived_evidence` replaces `files_changed` only
`if !derived.is_empty()`, and downgrades unsupported claims to `Blocked` only for
`Completed`. A handoff can therefore carry a fabricated `filesChanged` past
reconciliation with a clean tree and open a gate over nothing.

Amends contract item 1. `files_changed` is replaced with the derived list
**unconditionally, including an empty list**, and the "reported N files but the
tree is clean" mismatch is recorded for a handoff too. The fatal downgrade to
`Blocked` stays limited to `Completed`: downgrading a handoff would discard the
follow-up it asked for, and with an empty `files_changed` it can no longer open a
gate anyway.

- `a_fabricated_change_set_cannot_open_a_gate` — a handoff claiming files that
  Git does not show ends with an empty `files_changed`, a recorded mismatch, and
  no gate.

## A3 — a partial revision is not a completion candidate (was finding 3)

`opens_completion_gate` accepted every changed `needs_delegation`, including a
worker asking for another *implementation* worker. A verifier could then drive
that gate to `Verified` while the requested follow-up never ran, and the parent
notice carried neither `suggestedRole` nor `suggestedTask`, so the orchestrator
could not see what was actually asked for.

Replaces contract item 1's status rule. A handoff opens a gate only when the
worker's own `suggestedRole` is `verification` — "the implementation is done,
please verify" is a completion candidate; "I need another implementation worker"
is a partial revision and opens nothing. `suggestedRole` is always populated for
`needs_delegation` (`delegation.rs` derives it from `suggestedTask` and defaults
to `implementation`), so the default direction is the safe one.

A partial revision stays fully reported: the routing notice now carries
`suggestedRole` and `suggestedTask` and instructs the orchestrator to route the
follow-up the worker asked for. If verification is delegated anyway, A1's typed
rejection explains why it cannot bind.

- `only_a_handoff_asking_for_verification_opens_a_gate` — `suggestedRole:
  verification` opens a gate; `implementation`, `research`, and `planning` do not.

## A4 — adoption must not remove a checkout another worker is running in (was finding 4)

`worker_adoption::release_worktree` asks only whether the *implementation* worker
is reusable. A verifier bound to that path is a different session with no lease
on it, so adoption — or the `release_terminal_worktrees` maintenance pass — can
remove the verifier's checkout while verification is reserved or running.
`git::safe_remove_worker_worktree` refuses a dirty tree, which hides the bug in
the reported scenario but not once the work is committed.

New clause. `release_worktree` retains the worktree while any **other** live
session is bound to that path in `worker_runtime`, and records
`worker.worktree_retained` with the borrower named. This covers both a reserved
verifier (bound at launch, session `starting`) and a running one.

- `a_worktree_another_live_worker_runs_in_is_retained` — adoption settles the
  binding but leaves the directory in place while a live verifier is bound to it,
  and collects it once that verifier is terminal.

## A5 — only a live gate is a verification target (was finding 5)

`verification_target_path` filtered `status IN ('verifying','changes_requested',
'failed')` and ordered within that filter, so it could hand back a `failed`
attempt — which `settle_verification_result` treats as terminal and refuses to
re-open — and could select an *older* failure even when a newer terminal attempt
existed.

Amends contract item 4. The newest attempt for the session is selected first,
with no status filter, and a target is returned only when that attempt is
`verifying` or `changes_requested`. Anything else is `None`, which A1 reports as
the typed unroutable reason.

- `only_the_newest_live_attempt_is_a_verification_target` — a newer `verified`,
  `superseded`, or `failed` attempt hides an older `verifying` one, and a `failed`
  newest attempt is never a target.
