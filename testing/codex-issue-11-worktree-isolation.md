# codex/issue-11-worktree-isolation — Test Contract

## Functional Behavior

- Child writer worktrees are created from the task worktree's current commit in a deterministic task-scoped namespace, with a dedicated worker branch.
- Path ownership checks reuse the policy engine's normalized-pattern implementation. Disjoint writers may use separate child worktrees; overlapping or invalid ownership patterns are rejected before concurrent worktree creation.
- Clean Git checkpoints expose serializable metadata tied to a session-forest checkpoint entry: session ID, forest entry ID, commit, branch, and timestamp. Dirty worktrees cannot be recorded as clean checkpoints.
- Worker changes can be inspected without mutating the task worktree, then integrated only into a clean, inactive task worktree. Conflicted integration is aborted and returns an error.
- Worker worktrees can be removed only when clean and when their session is inactive. Dirty or active worktrees remain intact.
- Conversation-only branching performs no Git command or filesystem mutation. Conversation + child-worktree, restore-clean-checkpoint, and extract-worker-changes are explicit, distinct operations.
- Restoring a recorded checkpoint requires a clean, inactive task worktree and restores tracked index/worktree content without moving the current branch pointer.

## Unit Tests

- `normalized_overlap_is_shared_with_policy` — Git coordination delegates overlap decisions to the policy normalizer, including glob, nested, disjoint, and invalid patterns.
- `checkpoint_metadata_requires_clean_tree_and_forest_entry` — clean metadata is tied to a forest checkpoint entry; dirty and incomplete requests fail.
- `conversation_branch_only_has_no_filesystem_effect` — explicit conversation rewind leaves HEAD, status, and tracked contents unchanged.
- `dirty_or_active_worker_removal_is_blocked` — both safety conditions reject removal and retain the child worktree.
- `restore_checkpoint_requires_clean_inactive_tree` — restore is blocked for dirty/active trees and does not move the branch pointer.

## Integration / Functional Tests

- `disjoint_workers_create_integrate_and_remove_round_trip` — two disjoint writer worktrees branch from one task worktree, commit independently, expose inspectable change sets, integrate cleanly, and are safely removed.
- `overlapping_workers_are_rejected_before_creation` — an existing active writer with overlapping ownership prevents a second child worktree; a disjoint writer is allowed.
- `conflicting_integration_aborts_cleanly` — conflicting worker integration returns an error and leaves the task worktree outside a merge with its original HEAD/content intact.

## Smoke Tests

- `cargo test git::tests` passes.
- Full `cargo test` passes.
- `cargo check` passes.
- Existing frontend `bun run test`, `bun run check`, and `bun run build` pass because public runtime/UI behavior is unchanged.

## E2E Tests

- Fixture Git repositories exercise real `git worktree`, commit, diff, merge, restore, and removal commands without requiring provider processes or network access.

## Manual / cURL Tests

- N/A — this issue provides a tested Rust coordination API; runtime supervisor wiring is intentionally left to the owning supervisor change to avoid colliding with issue #7.
