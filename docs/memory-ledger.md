# Account memory ledger

“About me” memory lives in `memory_records` under the named scope `account:local`. Records can be saved explicitly or extracted from a visible chat under the configured Memory mode. The scope key is written by the server; it is never SQL NULL, never inferred from a missing workspace, and never `legacy:global`.

This is a different product from:

- **Session recall** — FTS5 over one chat’s forest, keyed by session id. See [session-forest.md](./session-forest.md#session-recall).
- **Router learning** — pass / latency / cost for `workspace:{id}` only. Direct chats are out of the router. Memory jobs do not enter the learning router. See [adaptive-learning.md](./adaptive-learning.md).

## What the ledger stores

One table, `memory_records`: id, `scope_key`, kind (`preference` / `fact` / `decision` / `constraint`), body, provenance, status, optional `source_session_id` (no foreign key — sessions can vanish), the revision chain (`supersedes` / `superseded_by`), the extractor’s trust fields (`confidence_bps`, `rationale`), the validity interval, an optional expiry, an optional conflict group, and timestamps. A separate FTS5 index carries active bodies only, maintained by trigger.

Provenance is `user_explicit` for a save or an edit, `model_proposal` for something the extractor suggested, `agent_proposal` for something an agent proposed with `memory_propose`, and `model_consolidation` for a record the consolidation job produced by merging or correcting existing ones.

Status is `active`, `proposed`, `rejected`, `superseded`, `expired`, or `deleted`. Only `active` lists, searches, and reaches a packet.

## How memory gets in

An explicit save: protocol `memory/save_memory_record` or slash `/pin <text>`. Saving text an active record already holds (the same once case, whitespace and edge punctuation are folded) returns that record and writes nothing, so a double click is one pin and a full scope does not refuse a pin it already has. "Remember this" on an assistant reply never saves: it opens Memory with the reply as a draft for the person to cut down to the claim worth keeping. Listing is `memory/list_memory_records` with a required `scopeKey`, or `/pins`. Forgetting is a tombstone (`status=deleted`) via `memory/delete_memory_record` or `/unpin <id>`. `/pin`, `/pins`, and `/unpin` are Bridge-handled and never auto-switch harness. They are not Claude’s `/memory`.

Or an extracted candidate. After each finished turn of a visible chat, extraction replays a bounded digest through a hidden tool-free session on the chat's own harness and model — or on a helper pinned in Memory settings. The user controls one of three modes:

- `remember` (**Manual only**) stops future extraction. Existing active extracted records remain active; new memory is saved only through explicit pins.
- `propose` (**Review first**, the default) queues each validated candidate that fits the scope budget as `proposed`; it becomes active only after approval.
- `auto_apply` (**Automatic**) first writes the same gated proposal, then promotes it through the normal `proposed -> active` transition only when it is at least 90% confident, cites a visible user message from the same chat, and is grounded beside an explicit durable cue in the same unquoted clause. Temporary or retracted wording, exact or likely matches to forgotten memory, and likely conflicts with active, pending, or same-batch memory stay out of automatic activation. Other validated candidates that fit the scope budget stay proposed for review. Invalid, unsafe, duplicate, and over-budget output is refused rather than queued.

Or an agent proposal. Every chat that loads the `bridge` MCP server gets `memory_search`, and a top-level chat also gets `memory_propose`, on the same per-chat token as the notes tools. A proposal runs the extraction gate below plus a required rationale, lands `proposed` / `agent_proposal` with the session id, and is never activated by any mode: the person approves it or it expires. Manual only refuses it, a chat may write at most five, and a repeat answers with the existing record. Bridge Notes are documents, not memory; the `notes_create` description says so. Such a chat's session-context frame also carries one line naming the two tools as Bridge memory, to use over any other connected memory tool: with only the tool descriptions, an agent searching its deferred tools for "memory" picked another server's tools about half the time, and nothing written there reaches the other harnesses.

The extractor cannot supply its own status, scope, or provenance. Bridge snapshots the mode when it enqueues a run and checks the current mode again before promotion, so a setting change can only downgrade automatic promotion to review, never escalate a review run. See [`testing/feat-memory-extract.md`](../testing/feat-memory-extract.md).

The server always writes `account:local`. The client cannot pass a scope on save. Empty, whitespace, and credential-shaped bodies are rejected.

## One claim, one record

`memory_similarity` is the single definition of "the same claim": equal normal forms, or, when both sides have at least three content terms, a Jaccard overlap of 80% or more. Extraction and agent proposals refuse a candidate that repeats an active, proposed or rejected record (a rejected claim does not come back with a word changed) or another candidate in the same batch. It is lexical on purpose, so every writer and the packet agree and a test can pin it; paraphrases need a model and are not claimed.

## Validity is an interval, not a flag

Every record carries `valid_from`, the instant its claim began to hold, and `valid_to`, the instant it stopped. An active record’s end is open. A record that never reached active — a proposal, a rejection — carries an empty interval where `valid_to` equals `valid_from`, so it was never true at any instant.

Superseding closes the predecessor exactly where the successor opens: one instant, no gap in which neither held and no overlap in which both did. A tombstone closes the interval at the tombstone. Nothing deletes a row, so a superseded or expired record stays queryable as history and the tombstone remains the only path to removal; a tombstoned body is the one thing history does not hand back, because forgetting it is what the user asked for.

`memory/list_memory_records_as_of` reads a scope at an instant and returns exactly the records whose half-open interval `[valid_from, valid_to)` contains it. Read as of now, that is the active set, which is why nothing that already read the ledger changed behaviour.

## Expiry

A record may carry `expires_at`. Passing it moves the record to `expired` and closes its interval **at the expiry**, not at the moment the sweep happened to run. Expiry is applied by a sweep that takes the current time as an argument and never reads a clock inside a query, so the boundary is exact: a record expiring at the sweep instant is expired and one expiring after it is not.

An expired record leaves the FTS index through the same trigger every other status change uses, so it cannot reach a packet; the packet excludes it by its own code rather than reporting it as superseded or deleted. Expiry is a lifecycle transition, not a deletion — body, provenance and interval all survive, so the user can see why a record stopped applying. A record with no expiry never expires, and an explicit save defaults to none.

## Proposals expire

A proposal that waits 30 days (`PROPOSAL_TTL_DAYS`) for review is swept to `expired` by the same maintenance tick that applies record expiries. It never held, so its interval stays empty and no as-of read returns it. It leaves the review queue and the budget, so a queue nobody opens drains instead of holding the scope at its ceiling. The Memory screen says proposals expire, and the sidebar's Memory row counts what waits for review.

## Conflict groups

Records that make competing claims about one subject share a `conflict_group`. At most one member is active at a time: activating a member closes whichever member was active, at exactly the instant the survivor continues from, and the loser is marked superseded by the survivor. A packet built from a scope containing a group carries at most one member of it, so two contradictory facts can never be injected together. Automatic extraction also applies a conservative lexical conflict guard before promotion; a possible conflict stays in review rather than bypassing this machinery. A group with no active member — every member expired or rejected — is a legible state, not an error: the scope simply has no answer for that subject.

## A bounded scope refuses rather than evicts

Each scope carries a budget, `maxRecords`, counting everything it holds: active records plus proposals still waiting for a decision. A write that would exceed it fails, naming the budget and what is held, and leaves the scope exactly as it was. An explicit save is refused with a message the user can act on; an extraction proposal is refused before it is stored and the run records that it was refused rather than dropping it in silence.

A full scope also stops extraction before it spends anything: a finished turn enqueues no run, and a run queued before the scope filled settles `cancelled` with detail `scope_full` instead of reaching an adapter. The Memory screen shows the full state with the held and budget numbers.

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

## The packet

The packet is built once at session start and goes in the session-context frame in the conversation tail (never the system prefix, so it cannot disturb the provider's prompt cache). It ranks active records by trust (typed pins first), then kind (constraint, preference, decision, fact), then confidence, then recency, and puts every record that fits one line ahead of every record that would need clipping. No line runs past 320 characters: a longer body is clipped with a marker that names `memory_search`, which returns the full text. A record that repeats one already selected is excluded with code `near_duplicate`, at most one member of a conflict group is carried, and the lines stop at 4,000 characters. Every packet writes one audit row naming what was selected and why everything else was left out.

`memory_evals::packet_eval` gates this against labeled ledgers in `testing/evals/memory-packet/`, including one shaped like the failure that motivated it: long assistant replies saved as pins crowding out short constraints.

## What was dropped

`task_knowledge` never had a production reader (only a round-trip unit test). Schema 31 drops that table **without copying rows**. A versioned data migration would have invented a contract for zero dependents.

## Not in this slice

Workspace-scoped memory, a consolidation UI, import/export, a scored replay benchmark for consolidation quality, routing consolidation through the learning router, mixing pins into the learning router, relevance ranking of the packet against the session's objective, a model-backed curator for paraphrase duplicates, and extraction for chats on harnesses that cannot run a tool-free turn.
