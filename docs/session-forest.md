# Session forest

Bridge stores conversation history as immutable entries in `session_entries`. Each entry belongs to one session, has an insertion-order `sequence`, and optionally points to a parent entry. `session_heads.active_entry_id` selects the active leaf.

This is [durable local history](local-history.md), not a tamper-proof or replicated evidence ledger.

```text
root → user → assistant → checkpoint
                └──────→ alternate user → assistant  (active)
```

Moving the head changes only the active conversation branch. It never rewinds a worktree, commit, or filesystem change. Appending after moving the head creates a new branch and leaves the previous entries intact.

Every controller append stamps the entry with the repository `HEAD` and a deterministic hash of the full porcelain dirty state. Forest snapshots compare the selected entry's stamp with the current worktree. A mismatch is surfaced as conversation/file divergence; legacy unstamped entries and non-repository sessions remain explicitly unknown. This controller-owned stamp is stripped before entries are projected into agent context.

## Stored entry shape

- `id`: immutable entry identity.
- `session_id`: owning adapter session.
- `parent_entry_id`: previous entry on this branch.
- `sequence`: deterministic insertion order across every branch in the session.
- `semantic_schema_version`: explicit persisted-event contract version. New writes use v2; projection supports v2 and N-1 v1, while unknown future versions fail closed.
- `kind`: provider-neutral semantic kind such as `user.message`, `tool.completed`, `checkpoint`, or `worker.result`.
- `payload`: semantic content plus provider metadata when needed for inspection.
- `context_visibility`: whether the projector may include the entry in restored context.

The React UI requests a forest snapshot and walks from the active leaf to the root. Raw provider notifications stay inspectable but collapsed. The legacy linear `agent_events` table is removed at schema version 6; normalized adapter events append directly to the forest.

## Three independent trees

Bridge deliberately keeps these separate:

1. Workspace tree: repository → task worktree → optional worker worktree.
2. Agent tree: orchestrator → policy-authorized workers.

Worker-result entry IDs are also durable evidence references. A later sibling worker receives the exact validated typed payload resolved from the parent's active branch rather than relying on an orchestrator paraphrase; active-branch membership and session ownership are checked on every resolution.
3. Conversation tree: immutable entries → active branch.

An operation on one tree does not imply a matching operation on another.
