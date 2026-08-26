# fix/327-verify-orchestrator-edits — Test Contract

Issue: https://github.com/Atharva-Kanherkar/bridge-harness/issues/327
("Verification worker fails to start after orchestrator self-implements a change
— `implementation_revision_unavailable`")

## The bug

The orchestrator made a small implementation change directly in its own turn
(no implementation worker was ever delegated), then delegated a verification
worker. The verifier failed to start:

> no implementation revision is recorded for this task
> (`implementation_revision_unavailable`) …

An implementation revision is an `eval_attempts` row, and it is created only by
`completion::create_from_worker_result`, which fires exclusively when a
*delegated implementation worker* reports a gate-opening typed result. Edits the
orchestrator makes itself never open a gate, so the verifier launched next has
nothing to bind to. Two defects combine:

1. **No self-recording path (root cause).** There is no code that opens an
   `eval_attempts` row from the orchestrator's own worktree state. Real work sat
   on disk while the completion machinery had no record of it.
2. **Late, misworded rejection.** The bindability check runs in
   `live_turn` *after* a worker has already been reserved, so the failure
   surfaces as a worker-launch failure card. Its remediation text ("adopt or
   commit the implementation **worker's** worktree changes") assumes an
   implementation worker exists — here none ever did, so the message is not
   actionable.

## Functional Behavior

- Delegating verification after the orchestrator edited files directly in its
  own checkout opens a completion gate automatically, right before the worker
  reservation is made:
  - The gate is stamped from Git (`repository_stamp`: HEAD + dirty digest) of
    the parent session's checkout (`COALESCE(s.cwd, w.path)`).
  - Changed paths come from Git (`git::derive_repository_evidence` dirty paths),
    never from orchestrator claims.
  - Acceptance criteria and verification commands come from the verification
    directive itself (the criteria the orchestrator asked to have checked).
  - `implementer_family` records the parent session's harness.
- If an open gate already exists (`verifying`/`changes_requested`), nothing new
  is written — existing behavior preserved.
- If the newest attempt matches the current stamp exactly, self-recording
  dedupes instead of stacking duplicate gates.
- If there is genuinely nothing to verify against — no open gate AND a clean
  checkout — the delegation is rejected **before any worker is reserved**, with
  a reason that distinguishes the two remediations:
  - "no implementation work is recorded at all" → delegate an implementation
    worker or make the change first;
  - an implementation worker's changes awaiting adoption → adopt/commit them.
- The post-reservation bindability check stays as a backstop (unchanged
  semantics); with the pre-reservation step in place it only fires on races.
- Not in scope: changing what counts as a valid revision once one exists, or
  the worker Git-evidence reconciliation (`live_turn.rs` ~6590).

Deliberate limitation (documented, not fixed): committed-but-uncommitted-to-any-
base orchestrator commits are not detected — detection covers uncommitted
worktree state, which is how orchestrators make small edits today.

## Unit Tests

Rust (`bridge-core/src/completion.rs`):

- `orchestrator_edits_open_a_verification_gate_without_a_worker` — dirty temp
  git repo wired as the parent's workspace path; verification directive with
  acceptance criteria; `create_from_orchestrator_edits` returns a summary whose
  verdict is `verifying`; `verification_target_path` now returns the checkout
  path; the attempt carries the current HEAD and digest (reproduces the issue).
- `a_clean_checkout_records_no_orchestrator_revision` — clean repo,
  `Ok(None)`/`Missing`; zero rows appear in `eval_attempts`.
- `self_recording_dedupes_while_the_stamp_is_unchanged` — calling ensure twice
  over the same tree state returns `Existing` the second time and leaves one
  attempt row, not two.
- `an_open_gate_is_left_alone_by_self_recording` — a pre-existing open gate is
  returned untouched (`Existing`) even when the directive could record another.
- `missing_target_reason_names_both_remediations` —
  `verification_target_unavailable_reason()` mentions both "delegate an
  implementation worker" and "adopt … worker('s) … changes".

## Integration / Functional Tests

- Existing `completed_implementation_opens_a_private_verification_gate` and the
  whole worker-path suite must keep passing — delegated-worker gating is
  untouched.
- Existing `verification_binding_tests` (abort/backstop behavior) must keep
  passing.
- `an_unroutable_verifier_is_aborted_without_a_gate` keeps asserting zero
  `eval_attempts` rows after an abort — the backstop still writes no gate.

## Smoke Tests

- `cargo test -p bridge-core completion::` green, including new tests.
- `bun run test` (vitest + cargo) green.
- `bun run build` and `bun run check` green.

## E2E Tests

N/A — no automated E2E harness for the desktop app. Manual repro stands in.

## Manual / cURL Tests

Repro of the original issue: in a Bridge task conversation, ask the orchestrator
to edit a file directly ("change X in src/foo.rs"), then delegate verification
("verify this"). Before the fix: "Worker failed to start —
implementation_revision_unavailable". After the fix: the verifier launches,
bound to the task checkout, and the completion gate shows the self-recorded
revision. Second repro (still-failing case): delegate verification in a fresh
conversation with no edits anywhere — rejection arrives as a delegation
rejection notice naming the missing implementation work, with no worker card.
