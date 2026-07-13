# Session forest

Bridge stores conversation history as immutable entries in `session_entries`. Each entry belongs to one session, has an insertion-order `sequence`, and optionally points to a parent entry. `session_heads.active_entry_id` selects the active leaf.

```text
root → user → assistant → checkpoint
                └──────→ alternate user → assistant  (active)
```

Moving the head changes only the active conversation branch. It never rewinds a worktree, commit, or filesystem change. Appending after moving the head creates a new branch and leaves the previous entries intact.

## Stored entry shape

- `id`: immutable entry identity.
- `session_id`: owning adapter session.
- `parent_entry_id`: previous entry on this branch.
- `sequence`: deterministic insertion order across every branch in the session.
- `kind`: provider-neutral semantic kind such as `user.message`, `tool.completed`, `checkpoint`, or `worker.result`.
- `payload`: semantic content plus provider metadata when needed for inspection.
- `context_visibility`: whether the projector may include the entry in restored context.

The React UI requests a forest snapshot and walks from the active leaf to the root. Raw provider notifications stay inspectable but collapsed. The legacy linear `agent_events` table is removed at schema version 6; normalized adapter events append directly to the forest.

## Three independent trees

Bridge deliberately keeps these separate:

1. Workspace tree: repository → task worktree → optional worker worktree.
2. Agent tree: orchestrator → policy-authorized workers.
3. Conversation tree: immutable entries → active branch.

An operation on one tree does not imply a matching operation on another.
