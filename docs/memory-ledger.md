# Account memory ledger

Explicit “about me” pins live in `memory_records` under the named scope `account:local`. That key is written by save; it is never SQL NULL, never inferred from a missing workspace, and never `legacy:global`.

This is a different product from:

- **Session recall** — FTS5 over one chat’s forest, keyed by session id. See [session-forest.md](./session-forest.md#session-recall).
- **Router learning** — pass / latency / cost for `workspace:{id}` only. Direct chats are out of the router. Memory jobs do not enter the learning router. See [adaptive-learning.md](./adaptive-learning.md).

## What this release stores

One table, `memory_records`: id, `scope_key`, kind (`preference` / `fact` / `decision` / `constraint`), body, provenance (`user_explicit` only), status (`active` or `deleted`), optional `source_session_id` (no foreign key — sessions can vanish), timestamps.

There is no embedding column, no TTL, no confidence, no revision chain, no retrieval audit, and no FTS on the ledger yet.

## How pins get in

Only an explicit save: protocol `memory/save_memory_record` or slash `/pin <text>`. Listing is `memory/list_memory_records` with a required `scopeKey`, or `/pins`. Forgetting is a tombstone (`status=deleted`) via `memory/delete_memory_record` or `/unpin <id>`. `/pin`, `/pins`, and `/unpin` are Bridge-handled and never auto-switch harness. They are not Claude’s `/memory`.

The server always writes `account:local`. The client cannot pass a scope on save. Empty, whitespace, and credential-shaped bodies are rejected. Pins are not injected into every turn.

## What was dropped

`task_knowledge` never had a production reader (only a round-trip unit test). Schema 31 drops that table **without copying rows**. A versioned data migration would have invented a contract for zero dependents.

## Not in this slice

Workspace-scoped memory, LLM extraction, a Memory UI product, prompt injection, import/export, mixing pins into the learning router.
