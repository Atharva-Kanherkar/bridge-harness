# feat/handoff-budget-and-async-switch-summary — Test Contract

Issue: https://github.com/Atharva-Kanherkar/bridge-harness/issues/529
"[4/6] Cross-harness handoff: model-window token budget instead of 8 KB cap;
take the switch summary off the critical path." Parent audit: #525, gaps G3 and G4.

Re-derived against current `main` (`2d4a9083`). Line numbers in the issue are
approximate; the functions named there all still exist under the same names.

## Functional Behavior

### G3 — the carried context is sized by the incoming model, not by 8 KB

1. **A window source exists.** `model_catalog::context_window_tokens(harness, model)`
   returns the incoming model's context window in tokens. Known families
   resolve from the model id (Claude 200k / `[1m]` 1M, GPT-5 family 400k,
   Gemini 1M, o-series 200k, gpt-4.1 1M, gpt-4o 128k); anything unknown, and a
   `None` model, returns `DEFAULT_CONTEXT_WINDOW_TOKENS = 128_000` — the same
   figure `restoration.rs` and `context_breakdown.rs` already assume, so the
   fallback changes nothing for a chat Bridge cannot classify.
2. **The budget scales with the window and is capped by the prompt compiler.**
   `restoration::restoration_budget_bytes(window_tokens)` is one eighth of the
   window expressed in bytes (4 bytes per token), clamped to
   `[MIN_RESTORATION_BUDGET_BYTES = 8_000, MAX_RESTORATION_BUDGET_BYTES = 96 KiB]`.
   The cap leaves headroom under `prompt_compiler::MAX_VARIABLE_SUFFIX_BYTES`
   (128 KiB) for the other variable sections (memory packet ≤ 4k chars,
   session capabilities). Concretely: 32k → 16 000 bytes, 128k → 65 536,
   200k → 98 304 (cap), 1M → 98 304 (cap).
3. **`restoration::checkpoint_context` is header + verbatim tail up to budget.**
   The header is unchanged in kind (the newest valid compaction's summary and
   decisions, else the latest `checkpoint` summary) and is never dropped.
   The tail is the projected conversation (`user.message`, `assistant.message`,
   `worker.result`, `checkpoint`, `compaction`, `branch.summary`,
   `handoff.brief`) walked newest-first, whole entries only, until the byte
   budget is spent. The fixed 12-line stop and the 8 000-byte tail cut are gone.
   If even the newest entry alone exceeds the remaining budget, its head is
   trimmed on a char boundary so the newest words survive. The envelope line
   "Bridge checkpoint-restoration context (stored history, not native provider
   resume):" is unchanged.
4. **The budget comes from the session's own row.** `checkpoint_context(db, session_id)`
   reads `sessions.harness, sessions.model` — after a switch commits, that is
   the incoming model — and derives the window from (1). The projection call
   uses the same window in place of the hard-coded 128 000.
   `checkpoint_context_with_window(db, session_id, window_tokens)` exposes the
   budgeted variant for callers that know a different target.
5. **Handoff briefs use the target's budget.** `handoff::carry_brief` and
   `carry_brief_in_transaction` size the projected text by the *target*
   session's model, and the `handoff.brief` payload gains a short `summary`
   field (first line of the carried context, ≤ 200 chars) for any UI card;
   `text` stays the full carried context and remains required.
6. **Raw tool logs stay out.** The existing test
   `checkpoint_projection_uses_active_semantic_entries_only` keeps passing
   unchanged: `tool.completed` payloads never appear in the projected text.

### G4 — the outgoing model's summary is off the switch's critical path

7. **`update_chat_model` commits the switch before any summary turn starts.**
   For a switch that does not resume natively: plan the change → plan the
   summary (a `compaction.requested` entry with `reason=before_downgrade` and
   `background=true`) → *detach* the outgoing runtime (removed from
   `core.adapters`, parked in `core.detached_summaries`, its reader thread left
   running) → settle → commit (`session.model_changed`) → return. Only after
   the commit does a background thread deliver the checkpoint prompt. Measured
   as the issue asks: in the audit events table, `session.model_changed` has a
   lower id than any `checkpoint.turn_started` for that session, and the
   invoke returns without polling.
8. **The summary runs under the controller's timeout, not the switch's.**
   `sessions::SWITCH_SUMMARY_TIMEOUT_SECONDS` is deleted along with the inline
   300 ms polling loop. The background waiter polls
   `switch_summary_outcome` until a terminal entry lands or
   `compaction_controller::CHECKPOINT_TIMEOUT_SECONDS` (30 s) elapses, then
   stops the detached runtime and forgets it. A timeout still records
   `compaction.failed` (that is the honest outcome) with reason text
   "model-switch summary timed out after the switch; stored history carried".
9. **Reader routing.** `spawn_reader_thread` checks `core.detached_summaries`
   per line: when the entry for this session matches the launch's process id
   and provider session id, frames go to `switch_summary::handle_detached_events`
   instead of `handle_agent_value`. The detached handler only: (a) parses an
   assistant `message.completed` through `CompactionController::handle_output`
   (Completed / Repair → resend prompt via the detached runtime / Failed);
   (b) on `turn.completed` without a reply, schedules the single repair or
   records the failure exactly as the main reader does; (c) never writes
   `sessions.status`, `active_turn_id`, or any conversation entry. If the
   detached reader exits (provider died), the failure "checkpoint turn ended
   because the adapter exited" is recorded and the detached entry removed.
10. **The new session's reader ignores the background request.**
    `PendingCompaction.background` is parsed from the payload. In
    `handle_agent_value`, `checkpoint_turn_active`, `is_checkpoint_reply`, and
    the post-turn repair/failure block all filter out background pendings, so
    the incoming model's first reply is neither suppressed nor parsed as a
    checkpoint. `CompactionController::begin` still refuses while *any* request
    is pending (a background summary blocks pressure compaction for ≤ 30 s).
11. **Landing rule.** When the summary validates: if no conversation entry
    (`CONVERSATION_KINDS`) has been appended after the request's own
    `compaction.requested` sequence, it is recorded as the normal
    `checkpoint` + `compaction` boundary, so the incoming model's cold start
    finds it as `restoration_context` (the "prepend" in the issue). If the new
    session has already produced conversation entries, it is recorded as a
    plain `checkpoint` entry (`provenance: agent`, `landing: late`) with no
    boundary move, and `pending_from_branch` treats a `checkpoint` newer than
    the request as terminal so the request settles. `switch_summary_outcome`
    reports both shapes as `Summarised`.
12. **A failed summary still yields a reconstructed checkpoint.** When the
    background outcome is `Failed` (invalid twice, timeout, adapter exit) and
    no conversation entry has landed after the request, the waiter calls
    `reconstruct_from_normalized_events_and_git_with_reason(..., BeforeDowngrade)`
    so a `provenance: reconstructed` checkpoint with `reason: before_downgrade`
    is recorded; `reconstruct_from_normalized_events_and_git` keeps its
    `PhaseBoundary` behaviour by delegating. If the new session has already
    spoken, reconstruction is skipped — a reconstructed boundary would
    summarise the new model's turns out of its own projection. The reader-side
    `should_recover_compaction` keeps excluding `BeforeDowngrade`; its comment
    is updated to say recovery for that reason is owned by the waiter.
13. **Failure paths keep the switch whole.** If no runtime is live, or the
    plan finds no summary worth asking for (below `SWITCH_SUMMARY_MIN_TOKENS`,
    not idle, already pending), the switch stops the adapter and commits
    exactly as before. If the commit fails after detaching, the detached
    runtime is stopped and the pending request cancelled with the commit
    error. Delivery failure of the background prompt cancels the request with
    "model-switch summary could not be delivered: …".
14. **The `session.model_changed` detail is truthful about timing.** When a
    background summary was started, the carry note reads
    "… ; a handoff summary from the previous model is being prepared in the
    background" in addition to the mechanical projection counts. Native
    switches are untouched.

Explicitly out of scope (unchanged): same-harness native resume (#527), the
compaction prompt wording and validation rules, `SWITCH_SUMMARY_MIN_TOKENS`,
the frontend transcript (no wire type changes), and any per-model window
plumbing into `ModelOption` (a static family table is the source for now).

## Unit Tests (Rust, `cargo test -p bridge-core`)

`model_catalog.rs`
- `context_window_tokens_resolves_known_families_and_defaults_conservatively` —
  claude-opus-4-6 → 200 000; `claude-sonnet-4-5[1m]` → 1 000 000; gpt-5-codex →
  400 000; gpt-4o → 128 000; unknown id and `None` → 128 000.

`restoration.rs`
- `restoration_budget_scales_with_the_window_and_respects_the_compiler_cap` —
  16 000 for 32k, 65 536 for 128k, cap for 200k and 1M; the cap is below
  `MAX_VARIABLE_SUFFIX_BYTES`; the floor holds for a 1k window.
- `a_forty_turn_conversation_keeps_its_recent_turns_verbatim` — 40 alternating
  user/assistant messages (~300 bytes each) on a Claude-model session: the
  context contains the last 20 messages verbatim and exceeds 8 000 bytes.
- `the_tail_is_cut_by_the_incoming_models_budget_not_a_line_count` — the same
  branch projected with a 32k window (16 000 bytes) contains fewer messages
  than with 128k, the newest message survives in both, and the text length
  never exceeds header + budget.
- `the_header_survives_when_the_budget_is_tiny` — a compaction summary and a
  huge newest message: the summary line is present and the newest message is
  head-trimmed on a char boundary (multibyte payload).
- `checkpoint_projection_uses_active_semantic_entries_only` — unchanged, must
  still pass.

`handoff.rs`
- `carry_handoff_projects_the_source_branch_into_the_target` — extended: the
  brief carries a `summary` string ≤ 200 chars and `text` still contains the
  carried content.

`compaction_controller.rs`
- `background_requests_are_parsed_and_only_a_matching_checkpoint_settles_them` —
  `begin_background` writes `background: true`; `pending()` reports it; a
  `checkpoint` appended after the request settles it; a `checkpoint` *before*
  the request does not.
- `late_landing_records_a_checkpoint_without_moving_the_boundary` — after a
  `user.message` newer than the request, `handle_output` with valid JSON
  appends `checkpoint` (landing=late) and no `compaction`; the projection's
  `restoration_context` is unchanged and `pending()` is `None`.
- `reconstruct_with_reason_records_before_downgrade` — the reconstructed
  checkpoint carries `reason: before_downgrade` and
  `provenance: reconstructed`; the un-suffixed function still writes
  `phase_boundary`.

`switch_summary.rs` (new module)
- `detach_moves_the_runtime_out_of_the_adapter_map_without_killing_its_reader` —
  after `detach`, `core.adapters` lacks the session, `core.detached_summaries`
  has it, and `stop` was not called on the runtime.
- `a_valid_reply_through_the_detached_handler_completes_the_request` — driving
  `handle_detached_events` with a valid checkpoint frame records
  `checkpoint` + `compaction` and leaves nothing pending.
- `a_turn_without_a_reply_schedules_one_repair_then_fails` — first
  `turn.completed` resends the repair prompt via the detached runtime; second
  records `compaction.failed`.
- `the_detached_handler_never_touches_session_status_or_turn_state` — status
  and `active_turn_id` are unchanged across `turn.started`/`turn.completed`.
- `a_failed_background_summary_reconstructs_only_before_the_new_model_speaks` —
  `settle_failed` reconstructs (before_downgrade) when no conversation entry
  follows the request, and skips when one does.

`sessions.rs`
- `a_cross_harness_switch_commits_before_the_summary_turn_starts` — hot
  `RecordingRuntime`, ≥ 1 500 conversation tokens, Claude → Codex stub:
  `update_chat_model` returns; `session.model_changed` precedes
  `checkpoint.turn_started` in the events table; the runtime is gone from
  `core.adapters` and present in `core.detached_summaries`; the pending
  compaction is `background`. The test then settles the request through the
  handler so the waiter thread exits promptly.
- `a_switch_with_nothing_to_summarise_stops_the_adapter_as_before` — below the
  token floor: no `compaction.requested`, adapter stopped, no detached entry.
- `switch_summary_outcome_tracks_the_pipeline_and_timeout_cancels_it` and
  `switch_summary_outcome_ignores_terminals_older_than_the_request` — still
  pass; the outcome also reports a late `checkpoint` as `Summarised`.
- `a_natively_resumable_switch_appends_no_handover_summary_through_the_api` —
  unchanged.

`live_turn.rs`
- `model_switch_failure_uses_projection_instead_of_racing_reconstruction` —
  unchanged assertions (BeforeDowngrade still excluded reader-side).
- `a_background_request_does_not_make_the_new_readers_reply_a_checkpoint` — with
  a background pending present, a normal assistant `message.completed` through
  `handle_agent_value` lands as `assistant.message`, not as checkpoint output.

## Integration / Functional Tests

- Full provider round-trip (real model producing the summary after the
  switch): N/A in cargo scope — no fixture spawns provider processes. Covered
  piecewise by the detached-handler tests plus the existing `handle_output`
  repair/complete coverage, which the background path reuses verbatim.
- `cargo test -p bridge-core` whole crate green (the reader loop, compaction
  controller, and restoration suites all touch this change).

## Smoke Tests

- `bun run check` green (tsc -b + cargo check --workspace).
- `bun run test` green (sidecar node:test + vitest + cargo test --workspace).
- `bun run build` green.

## E2E Tests

N/A — no automated desktop E2E harness exists in this repo. Manual script
below stands in.

## Manual / cURL Tests

Desktop app (`bun run tauri dev`), one chat:
1. Hold a 40-turn conversation on Codex that establishes several facts across
   the whole span (an early one, a middle one, a late one). Switch to Claude.
   The switch UI returns immediately (no multi-second spinner). Ask about the
   early, middle and late facts — the late and middle ones are answered from
   verbatim history, the early one from the summary header once it lands.
2. Open the transcript inspector: `session.model_changed` appears before
   `checkpoint.turn_started`; a `checkpoint` + `compaction` pair with
   `reason: before_downgrade` lands within ~30 s; no `compaction.failed`.
3. Repeat with the outgoing provider process stopped (cold): the switch
   commits instantly, no `compaction.requested` is written.
4. Send the first message to the new model *immediately* after switching, before
   the summary lands: the reply is normal prose (not swallowed), and the
   summary later appears as a plain `checkpoint` entry (`landing: late`).
5. `$codex what were we doing?` from a long Claude chat: the new chat's
   `handoff.brief` carries far more than 8 KB and includes the most recent
   turns verbatim; the transcript card shows only the short `summary`.
