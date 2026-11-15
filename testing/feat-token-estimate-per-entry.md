# feat/token-estimate-per-entry: Test Contract

Measurement only. No change to what any model sees, to prompts, tools or
compaction. No backfill: rows written before this branch keep `NULL`, and a
reader treats `NULL` as unknown.

## Functional Behavior

### Per-entry estimate (`session_entries.token_estimate`)

- Method: `bytes_div4_v1` (`prompt_compiler::TOKEN_ESTIMATE_SOURCE`), that is
  `ceil(model_readable_bytes / 4)`. No tokenizer.
- Computed once, at write time, inside `store::append_session_entry_tx`, from
  the payload the caller already holds. A caller that passes `Some(n)` keeps
  `n`. No branch load and no extra query on that path.
- Counts the text a model reads, never the JSON envelope, ids, `traceId`,
  `providerMeta`, `_bridgeRepoState`, or image attachments.
- Kinds and the fields they read:

| Kind | Bytes counted |
| --- | --- |
| `user.message`, `assistant.message`, `reasoning.completed`, `session.context`, `delegation.result` | root `text`, else `data.text` |
| `tool.started`, `command.started`, `file_change.started` (the call's input) | first present of `data.input`, `data.state.input`, `data.arguments`, `data.changes`, `data.command`, `data.patch` (strings by length, other JSON by compact serialized length) |
| `tool.completed`, `command.completed`, `file_change.completed` (the call's output) | root `text`, else `data.aggregatedOutput`, `data.state.output`, `data.output`, `data.content` (string, or the `text` of its blocks), `data.result` |
| `checkpoint`, `compaction` | `summary` + `decisions[]` + `filesTouched[]` + `openWork[]` |
| `branch.summary` | `summary` |
| `handoff.brief` | `text`, else `summary` |
| `worker.result` | `summary` + `suggestedNextAction` + `decisions[]` + `risks[]` + `remainingWork[]` + `filesChanged[]` |

- Any other kind (`turn.*`, `usage.updated`, `session.status`,
  `session.started`, `*.started` for messages and reasoning, `*delta`,
  `subagent.*`, `diff.updated`, `approval.*`, `permission.*`, `question.*`,
  `error`, `compaction.requested`, `compaction.failed`, model and effort
  changes, `artifact.*`, unknown kinds) stores `NULL`.
- An allowlisted kind whose recognised fields are all absent stores `NULL`
  (shape unknown), not `0`. A present but empty string stores `0`.
- `compaction_controller` keeps estimating from the full payload and ignores
  the stored column, so the switch-summary floor and `tokensBefore` do not
  move.
- Context projection (`context.rs`) uses the stored value when present and
  falls back to the payload estimate when `NULL` (already the case; locked by
  a test). The context breakdown reads the projection.

### Window used per request (`usage_ledger.context_used_tokens`)

- Meaning, the same for every harness: the last request's input, cache
  included, plus its output. Codex already writes `last.total_tokens`.
- Claude: the stream state keeps the last main-thread request's usage, read
  from `stream_event` `message_start` / `message_delta` and from `assistant`
  frames, taking each counter's maximum for one message id. Subagent frames
  (`parent_tool_use_id`) never touch the parent's figure. The `result`
  `usage.updated` event carries it as `data.lastRequest {model,
  contextUsedTokens}`. `usage::normalize_claude` puts it on the record for
  that model, or on the only record when there is one. Other models' records
  stay `NULL`.
  `contextUsedTokens = input_tokens + cache_read_input_tokens +
  cache_creation_input_tokens + output_tokens`.
- OpenCode: per step (one request) `context_used_tokens = input_tokens +
  output_tokens` (input is cache-inclusive as Bridge stores it, output
  includes reasoning). When the step has no input figure the column stays
  `NULL`, and `record_provider_usage` logs the reason once per session, not
  per frame.

## Unit Tests

- `entry_tokens::tests::prose_kinds_count_only_their_text` — `user.message`
  with a large `_bridgeRepoState`-like sibling and attachments counts only
  `text`.
- `entry_tokens::tests::tool_input_and_output_are_counted_once_each` —
  started counts input (object and string), completed counts output, not the
  reattached `command`.
- `entry_tokens::tests::output_falls_back_through_adapter_shapes` — Codex
  `aggregatedOutput`, OpenCode `state.output`, Claude content blocks.
- `entry_tokens::tests::bookkeeping_and_unknown_kinds_are_null`.
- `entry_tokens::tests::recognised_kind_with_no_fields_is_null_and_empty_text_is_zero`.
- `entry_tokens::tests::typed_summaries_count_their_fields`.
- `entry_tokens::tests::json_input_size_matches_compact_serialization`.
- `session_forest::tests::append_records_token_estimates_for_model_readable_kinds`
  — `user.message`, `tool.completed` and `command.completed` (adapter shape
  via `store::session_event`) store the expected estimate; a bookkeeping-only
  kind (`session.status`) stores `NULL`.
- `store::tests::explicit_token_estimate_wins_over_derived`.
- `context::tests::projection_uses_stored_estimate_and_falls_back_when_null`.
- `compaction_controller::tests::stored_estimates_do_not_move_compaction_figures`.
- `usage::tests::claude_last_request_sets_context_used_on_its_model`.
- `usage::tests::claude_without_last_request_leaves_context_used_null`.
- `usage::tests::opencode_step_reports_context_used`.
- `usage::tests::opencode_step_without_input_leaves_context_used_null`.
- `agent::tests::claude_result_carries_last_main_thread_request_usage` —
  subagent frames ignored; `message_delta` raises output.
- `policy::tests::opencode_missing_window_is_logged_once_per_session`.

## Integration / Functional Tests

- `agent::tests::claude_usage_window_fixture_reaches_the_ledger`: replays
  `testing/fixtures/claude-usage-window.json` through
  `normalize_claude_message_with_state`, records each `usage.updated` with
  `policy::record_provider_usage`, and reads `usage_ledger`:
  `context_used_tokens` is set on the main model's row and `NULL` on the
  helper model's row.
- `cargo test -p bridge-core` green.

## Smoke Tests

- `bun run check` and `bun run build` green.
- `bun run test` green (or each failure shown to be unrelated, with
  output).

## E2E Tests

- N/A for automation. Manual: run a debug build from this worktree on its own
  data dir, send one Claude and one OpenCode turn that runs a command, then run
  the queries below.

## Manual / SQL Tests

Per-kind report for one session, from the stored column only (no payload
parsed, served by `idx_session_entries_session_kind`):

```sql
SELECT kind,
       COUNT(*)               AS entries,
       COUNT(token_estimate)  AS estimated,
       SUM(token_estimate)    AS tokens
FROM session_entries
WHERE session_id = :session
GROUP BY kind
ORDER BY tokens DESC;
```

Window used per request since a date:

```sql
SELECT source, COUNT(*), COUNT(context_used_tokens), MAX(context_used_tokens)
FROM usage_ledger
WHERE source LIKE 'provider.%' AND created_at > :since
GROUP BY source;
```
