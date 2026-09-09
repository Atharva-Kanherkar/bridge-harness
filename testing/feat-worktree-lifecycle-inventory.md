# Contract: worktree lifecycle — inventory, reconcile, and retention policy

Branch `feat/worktree-lifecycle-inventory`. Closes the P0 and P1 halves of #574.
Locked before implementation.

## What this changes

Bridge creates worktrees in four places and reclaims them in one (workspace
archive, which no component calls). Nothing caps their count or size, nothing
reconciles Bridge's records against `git worktree list`, and a worker that
*stops* rather than settles keeps its checkout forever.

This change gives the coordinator a durable inventory and a retention policy:
every worktree Bridge creates gets a row with an owner, a boot and maintenance
pass reconciles rows against git and the filesystem, and a retention sweep
reclaims only what it can prove is safe — under a byte cap, a per-repo count
cap, and per-kind idle TTLs.

Out of scope, deliberately, for the follow-up PR: shared build caches, worker
scratch-branch deletion, the Settings storage surface, the archive-chat
affordance, and `bridge exec worktrees`.

## Non-negotiable safety rules

1. Never `--force`. A dirty worktree, a worktree with local-only commits, or
   one a live session is running in is retained with a recorded reason.
2. A `pending_adoption` binding is unadopted work. It is never swept, whatever
   its age, unless its diff against base is empty.
3. Bridge removes only worktrees it created and tracks, under its own
   namespace root. Anything else `git worktree list` reports is inventoried as
   `external` and never touched.
4. A worktree git can no longer read cannot be proven clean, so it is never
   auto-removed. It is recorded `unverifiable` and reported.
5. Classification re-runs at deletion time, not at scan time.
6. Branch refs are never deleted by this change.
7. Every removal writes an audit event naming path, branch, head, disposition,
   and bytes freed.
8. Git and filesystem work runs off the shared database mutex.

## Cases

### Inventory

- `registering_a_worker_worktree_records_its_owner_and_repo` — `prepare_isolated_worker`
  writes a `worktrees` row with kind `worker`, the owning session, the task
  worktree as repo root, and the branch it cut.
- `registering_an_orchestrator_worktree_records_its_owner` — the path that had no
  record at all now has one, keyed to the session that owns it.
- `registering_a_pull_request_checkout_records_the_workspace_node`.
- `re_registering_the_same_path_updates_rather_than_duplicates` — reuse (the PR
  checkout case) keeps one row.
- `migration_backfills_existing_worker_and_checkout_records` — a database
  carrying `worker_runtime`, `worker_worktree_adoptions`, and PR-checkout
  `workspaces` rows gains inventory rows for each distinct path, and an
  orchestrator session whose `cwd` sits under `worktrees/orchestrators/` is
  backfilled from that path alone.
- `migration_is_idempotent_across_reopens`.

### Reconcile

- `reconcile_marks_a_row_removed_and_prunes_the_registration_when_the_directory_is_gone`
  — one case, since the two effects share a cause: the row becomes `removed`,
  git's leftover registration is pruned, and `git worktree list` stops
  reporting it.
- `reconcile_adopts_an_orchestrator_checkout_at_its_own_depth` — added during
  implementation, and it caught a real defect: orchestrator checkouts nest
  twice (`orchestrators/<workspace>/<session>`), so walking one level found the
  workspace grouping directory instead of the checkout.
- `reconcile_adopts_an_untracked_directory_under_the_namespace_root` — a
  directory with no row, at the layout's own depth, becomes an `orphaned` row
  carrying the branch and head git reports.
- `reconcile_records_a_directory_git_cannot_read_as_unverifiable` — the live
  failure mode: the checkout survives but its admin directory is gone. It is
  inventoried, never removed, and never counted as reclaimable.
- `reconcile_leaves_external_worktrees_alone` — a worktree of the same repo
  outside the namespace root is counted `external`; no row claims it and
  nothing removes it.
- `reconcile_is_idempotent`.

### Classification

- `a_clean_worktree_with_no_commits_past_base_is_reclaimable`.
- `a_dirty_worktree_is_at_risk`.
- `a_worktree_with_local_only_commits_is_at_risk`.
- `a_clean_worktree_whose_commits_are_pushed_is_pushed_unmerged`.
- `a_worktree_a_live_session_runs_in_is_retained` — session `cwd` match.
- `a_worktree_a_live_worker_is_bound_to_is_retained` — `worker_runtime` match,
  covering the borrowed-verifier case.
- `a_pending_adoption_worktree_with_real_changes_is_retained`.
- `a_pending_adoption_worktree_with_an_empty_diff_is_reclaimable`.

### Retention sweep

- `the_sweep_reclaims_a_reclaimable_worktree_past_its_ttl`.
- `the_sweep_leaves_a_reclaimable_worktree_inside_its_ttl`.
- `the_sweep_never_removes_an_at_risk_or_retained_worktree` — and names each
  retention reason in its outcome.
- `exceeding_the_byte_cap_evicts_reclaimable_worktrees_inside_their_ttl`.
- `exceeding_the_per_repo_count_cap_evicts_oldest_reclaimable_first` — and
  removes only as many as the cap requires.
- `a_cap_breach_only_at_risk_worktrees_could_satisfy_is_reported_not_forced` —
  `over_budget_bytes` is non-zero and nothing is deleted.
- `the_pushed_unmerged_policy_is_honored` — `retain` (default) keeps it;
  `delete` reclaims it.
- `every_reclaim_writes_an_audit_event_naming_what_it_freed`.
- `a_removal_that_fails_does_not_stall_the_rest_of_the_sweep`.

### The stopped-worker leak

- `a_stopped_worker_whose_worktree_holds_nothing_settles_and_releases_it` — the
  live 3.9 GB shape: lifecycle `stopped`, binding `pending_adoption`, no change
  against base. It settles `empty` and the checkout is reclaimed.
- `a_stopped_worker_with_real_changes_keeps_its_worktree`, and
  `a_stopped_worker_with_uncommitted_changes_keeps_its_worktree` — the decision
  stays the user's.
- `a_stopped_worker_with_no_recorded_base_is_left_alone` — `record_binding`
  derives the base at launch, so a missing one means the derivation failed.
  Without it, emptiness cannot be proven, and an unprovable claim must not
  authorize a deletion.

### Creation gate

- `capacity_refuses_once_the_count_cap_is_reached` — the refusal names the
  budget; nothing is created.
- `a_repository_at_its_worktree_cap_queues_instead_of_spawning` — the policy
  input stops being a tautology, so an over-budget repository queues the
  delegation on `ChildWorktreeUnavailable`.

**Changed during implementation.** The contract asked the creation gate to
sweep before refusing. It does not: `prepare_isolated_worker` is called with the
shared database mutex already held, and a sweep runs git and walks directories,
so sweeping there would deepen exactly the problem #545 tracks. The gate stays a
cheap database-only capacity check; the sweep runs on the maintenance thread,
which frees capacity within a tick and lets the queued delegation proceed.

### Protocol

- `worktrees/list` returns the inventory with per-row disposition and size.
- `worktrees/usage` returns totals, the caps in force, and per-repo rollups
  (`usage_reports_totals_against_the_caps_in_force`).
- Both report the *last* assessment rather than recomputing: deciding a
  disposition costs several git invocations per checkout, and a read of the
  inventory must not shell out per row.
- Registry, typed-payload table, dispatch arm, Tauri command, and generated
  artifacts stay 1:1 — enforced by the existing drift gates.

## Review round: cases added after automated review of #577

Six findings from Codex and one from the Cursor security reviewer, each verified
against the code before fixing and each now pinned:

- `a_ready_chat_holding_a_live_adapter_keeps_its_checkout` and
  `a_ready_session_with_no_process_claim_does_not_pin_its_checkout` — liveness
  is a process claim, not a status string. `ready` means the provider is up.
- `a_reclaimed_checkout_is_restored_from_its_recorded_branch` and
  `restore_never_recreates_a_checkout_bridge_did_not_cut` — reclaiming must not
  cost a session its project.
- `the_removal_path_re_decides_before_deleting` — the full classification
  re-runs at deletion time, not just the cleanliness check git performs.
- `a_repository_exactly_at_its_count_cap_frees_one_slot_oldest_first` — the
  sweep clears a cap to strictly below it, or the creation gate's refusal can
  never be satisfied.
- `a_new_measurement_replaces_a_stale_larger_one` — a zero or smaller
  measurement is recorded, so freeing space stops being invisible to the cap.
- `reclaiming_an_empty_unadopted_checkout_settles_its_binding` — no pending
  adopt-or-discard decision is left pointing at a deleted directory.
- `a_symlink_in_a_layout_slot_is_never_adopted_or_reclaimed` — symlinks are not
  followed, and namespace containment is re-checked before every deletion.

## Gates

`bun run check`, `cargo test --manifest-path src-tauri/Cargo.toml --workspace`
(including `--no-run` after the signature changes), `bridge-protocol` drift,
`bunx vitest run`, sidecar `node --test`.
