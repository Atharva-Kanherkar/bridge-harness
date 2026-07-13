# Issue 34 Contract: Visible conversation/file divergence

- Every production session-entry append stores a controller-owned repository-state stamp derived from `HEAD` plus the full porcelain dirty-state hash. Non-repository sessions store an explicit `unavailable` stamp.
- Moving a conversation head never changes files. The returned forest snapshot compares the selected entry's stamp with the current worktree stamp and reports divergence when they differ.
- Legacy unstamped entries report an explicit unknown state rather than claiming alignment.
- The Deck shows: “This branch's context predates the current file state” whenever a selected stamped entry and current worktree differ.
- Repository stamps are storage metadata and never become agent-authored checkpoint or worker-result fields.

## Local refund checkpoint

Verify clean/dirty/head-change stamps, legacy behavior, conversation-only rewind, UI banner rendering, full frontend/Rust suites, type checks, and `git diff --check` before the issue-only PR.
