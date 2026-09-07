# fix/355-archive-reclaims-worker-worktrees — Test Contract

Locked before implementation. Issue #355.

## Background: what is actually broken

Issue #355 assumed archiving "writes rows and stops". That is wrong for the task
worktree — `BridgeCore::archive_workspace` already removes it and already refuses a
dirty workspace. The real leak is one level down:

- Worker (child) worktrees live at `namespace_root/<task>/<worker>`, **siblings** of
  the task worktree, not nested inside it (`git::worker_worktree_path`).
- `archive_workspace_records` deletes the workspace's sessions. `worker_runtime` and
  `worker_worktree_adoptions` reference `sessions(id) ON DELETE CASCADE`, and foreign
  keys are enforced, so those rows are destroyed by the archive.
- Nothing ever removes the worker worktree **directories**. After the archive, the
  directories remain on disk and stay registered in the repo's `.git/worktrees`, while
  the only records of where they were are gone — invisible, unreclaimable orphans.

So the fix is: archive must reclaim every worktree the workspace owns, and must refuse
rather than destroy when a worker worktree still holds unintegrated work.

## Functional Behavior

1. **Enumerate before mutating.** Archiving a workspace resolves the full set of
   worktrees it owns: the task worktree plus every worker worktree recorded in
   `worker_runtime.worktree_path` and `worker_worktree_adoptions.worktree_path` for
   sessions in that workspace. Duplicates collapse; empty/NULL paths are ignored; a
   path that no longer exists on disk is treated as already reclaimed, not an error.

2. **Classify each worker worktree, at removal time.**
   - *Reclaimable*: the directory is absent, or git reports a clean working tree.
   - *At risk*: the working tree is dirty (any uncommitted change, tracked or not).

3. **Refuse, do not destroy.** If any worker worktree is at risk, the archive fails
   with a message naming how many workers hold uncommitted work, and **nothing** is
   deleted — no rows, no directories. This matches the existing dirty-task-worktree
   refusal: Bridge never discards work the user has not committed.

4. **Reclaim on success.** When every worker worktree is clean, the archive removes
   all of them and the task worktree. Branch refs are never deleted — only checkouts.

5. **Atomicity preserved.** Worktree removal runs inside the existing archive
   transaction closure, so a failure to remove any worktree rolls the whole archive
   back (no half-archived workspace).

6. **Audit trail.** The `workspace.archived` event records how many worktrees were
   reclaimed, so history shows what the archive actually freed.

7. **Unchanged behavior.** Active sessions still block archiving; a dirty *task*
   worktree still blocks archiving; a clean workspace with no workers archives exactly
   as before.

## Unit Tests

In `src-tauri/bridge-core/src/workspaces.rs` (inline `#[cfg(test)] mod tests`):

- `archive_reclaims_worker_worktrees_alongside_the_task_worktree` — a workspace with
  two clean worker worktrees: after archive, both worker directories and the task
  directory are gone, and `git worktree list` in the repo no longer lists them.
- `archive_refuses_when_a_worker_worktree_has_uncommitted_work` — one clean worker,
  one dirty worker: archive returns an error naming uncommitted work; the workspace
  row, the session rows, **both** worker directories, and the task worktree all
  survive (full rollback).
- `archive_tolerates_a_worker_worktree_that_is_already_gone` — a recorded worker path
  deleted from disk beforehand does not fail the archive.
- `archive_records_what_it_reclaimed` — the `workspace.archived` audit event detail
  reports the reclaimed worktree count.

Existing tests that must stay green unchanged:
- `archive_workspace_removes_records_and_the_worktree`
- `archive_workspace_refuses_active_sessions_and_dirty_worktrees`
- `archive_workspace_rolls_back_when_worktree_removal_fails`
- `archive_workspace_does_not_remove_the_worktree_when_audit_fails`

## Integration / Functional Tests

N/A — no protocol surface changes. `workspaces/archive_workspace` keeps its exact
params and return type, so no generated artifact changes and no frontend change.
`cargo test -p bridge-protocol` must stay green as drift evidence.

## Smoke Tests

- `bun run check` (tsc -b + cargo check --workspace) green.
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core --lib workspaces::` green.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` green.
- `bunx vitest run --exclude "**/.claude/worktrees/**"` green (unchanged frontend).

## E2E Tests

N/A — archiving has no UI caller today (`api.ts::archiveWorkspace` is unreferenced by
any component), so there is no user journey to drive. Surfacing archive in the UI is
follow-up work and explicitly out of scope for this change.

## Manual / cURL Tests

No HTTP surface. Manual verification via the Rust fixtures above, plus a real-repo
sanity check that `git worktree list` shows no leftover entries after an archive that
included workers.

## Out of scope (deliberate, tracked separately)

- Reclaiming *already orphaned* worktrees from past archives (the backlog sweep):
  their DB rows are already cascade-deleted, so a sweep must scan `git worktree list`
  against Bridge's records — a different mechanism, worth its own change.
- Auto-reclaim settings toggle and the "pushed but unmerged" policy choice.
- A UI affordance for archiving.
