# fix/learning-dialog-truth — Test Contract

Written after the fact. The branch shipped without a contract, so this records
what its tests already enforce rather than proposing new behavior. The Learning
router dialog must show a fresh read, the real rollback target, and no schedule
write the user did not ask for.

## Refetch on learning-job-changed

- While open with a workspace, the dialog subscribes to `learning-job-changed`
  and refetches that workspace's learning state.
- Every read takes a generation from `learningReadGeneration` and is applied
  only if it is still the newest, so a slow earlier read cannot overwrite a
  newer one whichever resolves last.
- Merging a fresh read keeps the user's unsaved schedule edits.
- Tests: `refetches learning state when a learning job changes`, `a slower
  learning-state read cannot win`.

## Rollback target is the live predecessor

- `rollbackTargetVersion` is the predecessor of the live policy (canary, else
  active) in this workspace, not the latest run's `basePolicyVersion`, which can
  name the live version itself.
- No live policy, or a predecessor of 0 or NULL, means no target and no rollback
  button.
- Test: `rollback_target_is_live_predecessor_not_latest_run_base`.

## The schedule is written only when edited

- Save calls `updateLearningSchedule` only when enabled, cadence, or mode
  changed. `nextRunAt` and the disabled evaluator ceilings are not user edits.
- `update_schedule` applies a client `nextRunAt` only on the disabled-to-enabled
  edge. The transition test lives inside the UPDATE, so the runner keeps
  `next_run_at` while enabled and a stale dialog snapshot cannot rewind it.
- Closing the dialog drops unsaved schedule edits; reopening reloads.
- Tests: `update_schedule_does_not_rewind_next_run_at_while_enabled`,
  `a_runner_advance_survives_a_stale_dialog_save`,
  `enabling_schedule_accepts_client_next_run_at`, `treats schedule nextRunAt and
  evaluator ceilings as not user-editable`, `does not rewrite the learning
  schedule when save has no schedule edits`, `closing the dialog drops unsaved
  schedule edits`.

## Out of scope

Evaluator execution, spend and token ceilings, cross-workspace rollback,
provider-side memory.
