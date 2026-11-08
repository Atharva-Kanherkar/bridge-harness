# Contract: attention ledger in core

Branch `feat/attention-ledger`. Implements #905, slice 1 of #904.
Locked before implementation.

## What this changes

Bridge decides what deserves a person's attention in the desktop webview, by
diffing two session-list snapshots (`src/attentionEvents.ts`). Nothing else
can see those decisions, workers ping the person when they finish, and worker
approvals never set the worker's status to `waiting`, so they ping nobody.

This change moves the decision into bridge-core. Every durable forest entry
already goes through one function, `store::append_session_entry_tx`: adapter
events via `session_event_in_transaction`, and typed entries (delegation
launch approvals, `policy.rs`) via `SessionForest`. Imports and forks copy
history with direct inserts and never pass through it, so old history never
raises anything. Right after the entry is appended, a pure classifier decides whether
the event opens, closes, or does not touch an attention row, and the row is
written in the same transaction. Clients read the ledger through two new
methods and learn that it moved through one refetch hint.

No UI changes in this PR. The desktop keeps its current behaviour until #906.

## Functional behavior

Classification (`attention::classify`), for a session that exists and whose
kind is not hidden (`work_briefing_config::is_hidden_session_kind`):

| Forest kind | Status / condition | Top-level chat | Worker (`parent_session_id` set) |
|---|---|---|---|
| `turn.completed` | `completed`, `end_turn` | open `finished` | nothing |
| `turn.completed` | `failed`, or starts with `error` | open `failed` | nothing |
| `turn.completed` | `cancelled`, `interrupted`, anything else | nothing | nothing |
| `approval.requested`, `permission.requested`, `question.requested` | any | open `needs_you` | open `needs_you` |
| `approval.resolved`, `approval.settled`, `permission.resolved`, `permission.denied`, `question.resolved`, `question.rejected`, `delegation.approved`, `delegation.rejected` | any | close the open `needs_you` rows | same |
| anything else | | nothing | nothing |

- Hidden kinds and unknown sessions: nothing, for every forest kind.
- Restore, warm-up and resume emit no `turn.completed`, so they write nothing.
- `delegation.requested` means the launch was queued, not that it needs a
  person (`policy.rs`: `RouteDecision::Queue`), so it writes nothing. A launch
  that needs approval is written as `approval.requested`.
- The classifier returns before any SQL for every other kind, because this
  path runs for every streamed tool event.
- `needs_you` rows carry `approval_event_id` = the forest sequence of the
  request entry, so a client can resolve that approval without replaying.
- Opening a row supersedes every older open, unseen row of the same session
  (`superseded_by` = the new row's sequence). Newest per session wins.
- Closing sets `resolved_at` on every open `needs_you` row of that session.
- Severity: `finished` is `informational`; `failed` and `needs_you` are
  `actionable`.
- Copy is written server-side so every client says the same thing:
  - `needs_you`: title "Bridge needs you", body "<chat> · <Harness> is waiting for your input"
  - `failed`: title "Turn failed", body "<chat> · <Harness> ended this turn with an error"
  - `finished`: title "Turn completed", body "<chat> · <Harness> finished its turn"
  - `<chat>` is the session title, else its label, with the placeholder labels
    (`orchestrator`, `bridge orchestrator`, `new chat`, `chat`) shown as "New chat".
  - `<Harness>` mirrors `harnessLabel` in `src/utils.ts`.
- A forest write that rolls back leaves no attention row.
- An attention write that fails never fails the forest entry: it runs in a
  savepoint, is rolled back on its own, and is logged.
- Only `audience = 'human'` rows exist in this PR; the column is there for #911.

Wire:

- `attention/list_attention { afterSequence?, limit?, unseenOnly? } -> { items, latestSequence }`.
  Ascending by sequence, `afterSequence` exclusive, `limit` default 100, max 500.
  Superseded rows are excluded. `unseenOnly` excludes rows with `seenAt`.
  `latestSequence` is the ledger's max sequence (0 when empty), so a fresh
  client can start its cursor without paging.
- `attention/mark_attention_seen { sessionId } -> { updated }` stamps `seen_at`
  on every unseen row of that session and returns how many it stamped. (Not
  `mark_seen`: the wire command is the Tauri command name, and those are global.)
- Notification `attention/changed`, Transient, payload `null`. The event bus
  publishes it right after any durable `agent-event` (sequence > 0) whose kind
  is in the table above, so it always follows the commit. It is reconciled on
  lag like `memory-changed`.
- `mark_attention_seen` publishes `attention/changed` when it stamped anything.
- Protocol minor version 1.24, with a handshake note like the earlier bumps.

## Unit tests (bridge-core)

- `attention::tests::classify_table` — every row of the table above, for a
  top-level chat and for a worker.
- `attention::tests::hidden_and_unknown_sessions_write_nothing`
- `attention::tests::newest_row_supersedes_older_unseen_rows`
- `attention::tests::resolution_closes_open_needs_you_rows`
- `attention::tests::seen_rows_are_not_superseded`
- `attention::tests::copy_uses_title_label_and_harness`
- `attention::tests::list_pages_ascending_and_filters_unseen`
- `attention::tests::mark_seen_stamps_only_that_session`
- `store` test: `session_event_in_transaction` writes the row; rolling the
  transaction back leaves none (`forest_write_and_attention_row_commit_together`).
- `store` test: `a_failing_attention_write_never_costs_the_forest_entry`.
- `store` test: a typed `SessionForest` append of `approval.requested` writes a
  `needs_you` row (`typed_forest_approval_opens_needs_you`).
- `events` test: publishing an `Agent` event of a trigger kind also delivers
  `AttentionChanged`; a non-trigger kind or a sequence-zero frame does not;
  a lagged receiver gets `AttentionChanged` back from `reconciliation_events`.

## Integration / functional tests

- bridged dispatch: `attention/list_attention` and `attention/mark_attention_seen`
  round-trip through `dispatch.rs` (existing dispatch test pattern).
- Protocol gates stay green: `tsgen::checked_in_artifacts_match_the_contract`,
  `params_types_are_named_after_their_method`, params `deny_unknown_fields`
  strictness, typed-method totality, registry ↔ Tauri handler 1:1 in the
  shell crate, notification delivery classes pinned.

## Smoke tests

- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core attention`
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-protocol`
- `bun run check` and `bun run build`

## E2E tests

N/A for this PR. No UI consumes the ledger yet; #906 adds the desktop path
and its tests.

## Manual / socket tests

Against a daemon built from this branch with a short `--data-dir`:

1. Start a chat, send one prompt, let it finish.
2. Call `attention/list_attention {}` over the socket: one `finished` row for
   that chat, `latestSequence` equal to its sequence.
3. Trigger a permission request in a worker: one `needs_you` row for the
   worker with `approvalEventId` set; resolving it sets `resolvedAt`.
4. `attention/mark_attention_seen { sessionId }` returns `updated >= 1`; listing with
   `unseenOnly: true` no longer returns those rows.

## Review amendments (review 5817244094, and the core halves of 5817244230 and 5817244323)

Locked before the fixes; each fix lands in its own commit with a regression
test that fails without it.

1. **Hints for writes that publish no agent event.** Typed forest appends
   (a delegation scope approval from `policy::record_decision`) and other
   paths commit ledger rows without publishing an `agent-event`, so the bus
   never derives `attention/changed`. Every ledger mutation now bumps a
   revision in `attention_meta` inside the same transaction. The queued-input
   sweep (every 2 s) publishes `attention/changed` whenever the committed
   revision moved since it last looked. The bus-derived hint stays for the
   common path; the sweep is the guarantee. Test:
   `typed_approval_without_an_agent_event_still_hints`.
2. **Routing decisions do not answer approvals.** `delegation.approved` and
   `delegation.rejected` are ordinary routing records with no request id, so
   they no longer close anything. A scope approval is closed by the
   `approval.resolved` its resolution writes. Test:
   `routing_decisions_leave_a_pending_approval_open`.
3. **Checkpoint turns are maintenance.** While a checkpoint turn is active,
   the frames it keeps for the supervisor (`turn.completed`, `error`, ...)
   carry `data.bridgeCheckpointTurn = true`, and the classifier opens nothing
   for them. Test (real handlers): `a_checkpoint_turn_raises_no_attention`.
4. **Fatal errors without a boundary.** An `error` frame with status `failed`
   on a top-level visible chat opens `failed`, because ACP disconnects and
   others never send `turn.completed`. A `failed` row is not opened while the
   chat's newest visible row is already an unseen `failed` one, so a provider
   that sends both frames raises one failure. Tests:
   `a_fatal_error_without_a_boundary_raises_failed`,
   `error_and_boundary_for_one_failure_raise_one_row`.
5. **Questions are not approvals.** Every `needs_you` row stores its
   request's forest sequence in `request_event_id`, which resolution
   matching uses. `approval_event_id` (and the wire `approvalEventId`) is set
   only for `approval.requested` and `permission.requested`; a question is
   answered through `approvals/resolve_question`, so it gets no approval id
   and no Allow/Decline. Test: `question_rows_carry_no_approval_id`.
6. **Older clients decode newer kinds.** Wire `AttentionKind` and
   `AttentionSeverity` carry an `other` fallback (`#[serde(other)]`), so a
   client built against this version decodes kinds added later instead of
   failing the whole read. Test: `unknown_attention_kinds_decode_as_other`.

