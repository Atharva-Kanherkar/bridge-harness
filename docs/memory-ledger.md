# Account memory ledger

Explicit “about me” pins live in `memory_records` under the named scope `account:local`. That key is written by save; it is never SQL NULL, never inferred from a missing workspace, and never `legacy:global`.

This is a different product from:

- **Session recall** — FTS5 over one chat’s forest, keyed by session id. See [session-forest.md](./session-forest.md#session-recall).
- **Router learning** — pass / latency / cost for `workspace:{id}` only. Direct chats are out of the router. Memory jobs do not enter the learning router. See [adaptive-learning.md](./adaptive-learning.md).

## What the ledger stores

One table, `memory_records`: id, `scope_key`, kind (`preference` / `fact` / `decision` / `constraint`), body, provenance, status, optional `source_session_id` (no foreign key — sessions can vanish), the revision chain (`supersedes` / `superseded_by`), the extractor’s trust fields (`confidence_bps`, `rationale`), the validity interval, an optional expiry, an optional conflict group, and timestamps. A separate FTS5 index carries active bodies only, maintained by trigger.

Provenance is `user_explicit` for a save or an edit, `model_proposal` for something the extractor suggested, and `model_consolidation` for a record the consolidation job produced by merging or correcting existing ones.

Status is `active`, `proposed`, `rejected`, `superseded`, `expired`, or `deleted`. Only `active` lists, searches, and reaches a packet.

## How pins get in

An explicit save: protocol `memory/save_memory_record` or slash `/pin <text>`. Listing is `memory/list_memory_records` with a required `scopeKey`, or `/pins`. Forgetting is a tombstone (`status=deleted`) via `memory/delete_memory_record` or `/unpin <id>`. `/pin`, `/pins`, and `/unpin` are Bridge-handled and never auto-switch harness. They are not Claude’s `/memory`.

Or a reviewed proposal. After each finished turn of a visible chat, extraction (`propose` mode, the default) replays a bounded digest of that chat through a hidden tool-free session on the chat's own harness and model — or on a helper pinned in the Memory settings — and queues what it finds as `proposed` records. A proposal reaches a packet only once the user approves it; `remember` mode turns the run off. See [`testing/feat-memory-extract.md`](../testing/feat-memory-extract.md).

The server always writes `account:local`. The client cannot pass a scope on save. Empty, whitespace, and credential-shaped bodies are rejected.

## Validity is an interval, not a flag

Every record carries `valid_from`, the instant its claim began to hold, and `valid_to`, the instant it stopped. An active record’s end is open. A record that never reached active — a proposal, a rejection — carries an empty interval where `valid_to` equals `valid_from`, so it was never true at any instant.

Superseding closes the predecessor exactly where the successor opens: one instant, no gap in which neither held and no overlap in which both did. A tombstone closes the interval at the tombstone. Nothing deletes a row, so a superseded or expired record stays queryable as history and the tombstone remains the only path to removal; a tombstoned body is the one thing history does not hand back, because forgetting it is what the user asked for.

`memory/list_memory_records_as_of` reads a scope at an instant and returns exactly the records whose half-open interval `[valid_from, valid_to)` contains it. Read as of now, that is the active set, which is why nothing that already read the ledger changed behaviour.

## Expiry

A record may carry `expires_at`. Passing it moves the record to `expired` and closes its interval **at the expiry**, not at the moment the sweep happened to run. Expiry is applied by a sweep that takes the current time as an argument and never reads a clock inside a query, so the boundary is exact: a record expiring at the sweep instant is expired and one expiring after it is not.

An expired record leaves the FTS index through the same trigger every other status change uses, so it cannot reach a packet; the packet excludes it by its own code rather than reporting it as superseded or deleted. Expiry is a lifecycle transition, not a deletion — body, provenance and interval all survive, so the user can see why a record stopped applying. A record with no expiry never expires, and an explicit save defaults to none.

## Conflict groups

Records that make competing claims about one subject share a `conflict_group`. At most one member is active at a time: activating a member closes whichever member was active, at exactly the instant the survivor continues from, and the loser is marked superseded by the survivor. A packet built from a scope containing a group carries at most one member of it, so two contradictory facts can never be injected together. A group with no active member — every member expired or rejected — is a legible state, not an error: the scope simply has no answer for that subject.

## A bounded scope refuses rather than evicts

Each scope carries a budget, `maxRecords`, counting everything it holds: active records plus proposals still waiting for a decision. A write that would exceed it fails, naming the budget and what is held, and leaves the scope exactly as it was. An explicit save is refused with a message the user can act on; an extraction proposal is refused before it is stored and the run records that it was refused rather than dropping it in silence.

Nothing evicts on overflow. Silently discarding the oldest record is the failure mode where a user cannot tell whether memory was full, trimmed, or never written. A replacement is not growth, so editing a pin still works at the ceiling.

## Consolidation

Consolidation is opt-in per scope (`off` by default, `propose` to run it) and answers a closed vocabulary of operations against records that already exist, each naming a target:

| Operation | Effect |
|---|---|
| `merge` | Two or more targets are superseded by one new record. |
| `correct` | One target is superseded by a corrected claim. |
| `expire` | One target gets an expiry, derived by the gate from a horizon in days. |
| `group` | Two or more targets become a conflict group; one keeps answering. |
| `retire` | One target is tombstoned. Opt-in; refused, never downgraded, when removal is off. |
| `keep` | The target is left alone. Declining is a complete answer. |

Anything outside that vocabulary, a target that is not in the scope, a target that is not active, or an operation the settings do not permit is refused and counted; the rest of the batch still applies. A merge is expressed as a supersession of every record it replaces, so provenance and the interval chain survive it and every source stays reachable. A proposal cannot set a status, a provenance, a scope, an interval, or an expiry the gate did not derive.

The job sees the scope’s active records and nothing else: no session entries, no transcript, no other scope. It runs in a hidden bounded session with an empty tool scope, one turn, no repair, and a wall-clock cap, and settles exactly once with observed tokens and spend. Runs are debounced per scope — a new turn replaces the pending run rather than queueing a second, so consolidation never reads a conversation still in progress. Turning it off settles queued runs instead of executing them, and a scope with no remaining budget settles `skipped` without a model call.

Consolidation is bookkeeping rather than judgement, so the settings point at a harness and model instead of hardcoding one; the cheapest model the user has configured is usually the right choice. The expiry sweep is deterministic and runs regardless, so a scope that never turns consolidation on still expires records on time.

## What was dropped

`task_knowledge` never had a production reader (only a round-trip unit test). Schema 31 drops that table **without copying rows**. A versioned data migration would have invented a contract for zero dependents.

## Not in this slice

Workspace-scoped memory, a consolidation UI, import/export, a scored replay benchmark for consolidation quality, routing consolidation through the learning router, and mixing pins into the learning router.
