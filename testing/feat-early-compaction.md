# feat/early-compaction: Test Contract

Locked before implementation. Plan note: the comment "Plan note: ask the harness to compact earlier at turn boundaries" on the issue of this change. Stack: on `fix/claude-usage-turn-share`.

Bridge asks a top-level direct Claude or Codex chat to compact its own window at an idle turn boundary. The trigger fires when the window reading of that turn is at or above one threshold. The harness does the compaction with its own command. Bridge picks the moment, writes an attempt record, and settles the record. A global switch is off by default. With the switch off, no code of this change writes or sends.

## Functional Behavior

### Scope

- The trigger fires only for a chat of kind `direct`, with no parent, at depth 0, on harness `claude` or `codex`.
- Orchestrators, workers, workflow sessions, sessions with a parent, and OpenCode get named suppressions. Each one is a follow-up in the plan note.
- Bridge sends no focus text. There is one threshold, no second threshold, and no phase signal. Bridge reads no pull request or worker state for the decision.

### The rule

- `early_compaction::rule::decide` is pure. It reads no file, no clock and no database. The same inputs give the same decision.
- `early_compaction::rule::precheck` holds the suppressions that need only memory. `decide` calls `precheck` first.
- The rule returns `Fire(window_over_threshold)` or `Suppress(name)`. The first match in this order wins. Drop means that the due mark goes. Keep means that the due mark stays until `WARM_LIMIT`.
  1. `disabled`: the global switch is absent or false. Drop.
  2. `opted_out`: the chat has an opt-out row. Drop.
  3. `harness_not_allowed`: the harness is not `claude` or `codex`. OpenCode gets this name. Drop.
  4. `not_direct`: the kind is not `direct`, for example an orchestrator, a workflow session or a worker. Drop.
  5. `has_parent`: `parent_session_id` is set, or the depth is above 0. Drop.
  6. `unsupported`: the runtime reports `NativeCompaction::Unsupported`. Drop.
  7. `no_runtime`: the adapter map has no runtime for the chat. Bridge never starts a process to compact. Drop.
  8. `launch_mismatch`: the mark has no launch, or `reader_launch_is_current` is false for the launch of the mark. Drop.
  9. `launch_attempt_failed`: an earlier attempt on this launch ended as `failed`, `unsupported` or `unsettled`. Drop.
  10. `launch_attempt_ineffective`: the next reading of an earlier `compacted` attempt on this launch was at or above `THRESHOLD_TOKENS`. Drop.
  11. `attempt_open`: the chat has an open attempt. Keep.
  12. `too_soon`: the mark is younger than `IDLE_DELAY`. Keep.
  13. `cache_cold`: the mark is older than `WARM_LIMIT`. Drop.
  14. `turn_active`: the status is not `ready` or `idle`, or `active_turn_id` is set. Keep.
  15. `approval_open`: the chat has an open approval request. Keep.
  16. `tool_open`: the chat has a started tool with no completion. Keep.
  17. `children_live`: the chat has a worker result or delivery that is not done, a live harness subagent, or a background task in progress. Keep.
  18. `queued_input`: the chat has queued input. Keep.
  19. `state_unknown`: a state query failed or hit its cap. Keep.
  20. `no_reading`: no reading of this launch came after the turn end yet. Keep.
  21. `reading_mismatch`: the latest reading belongs to another launch, model, harness or turn. Drop.
  22. `below_threshold`: `used_tokens` is below `THRESHOLD_TOKENS`. Drop.
- `launch_attempt_ineffective` stops a loop. A chat with a large fixed part in its window stays above the threshold after each compaction.
- The window value is `used_tokens` of the bound reading. The threshold is a count of tokens, not a percent.
- Provisional constants: `THRESHOLD_TOKENS` 100,000, `IDLE_DELAY` 20 s, `WARM_LIMIT` 100 s, `SETTLE_LIMIT` 300 s, `NEXT_READING_LIMIT` 30 min, `TOOL_SCAN_CAP` 2,000 entries. A new value is a change of one constant.
- `WARM_LIMIT` stays below `IDLE_TTL` of `runtime_budget` (120 s).

### The reading

- The sweep reads the latest `context_readings` row of the chat with `ORDER BY id DESC LIMIT 1`.
- The row binds to the mark when three conditions are true. Its `provider_session_id`, `harness` and `model` equal the current values of the session and of the mark. Its `created_at` is at or after `started_at` of the launch. Its turn matches.
- Codex: the `turn_id` of the row equals the turn id of the mark.
- Claude: the source of the row is `claude.context_usage`, its `turn_id` is null, and its `created_at` is at or after the turn end. The sidecar reads the window after the `result` frame, so the row has no turn id.
- A row of the same launch from before the turn end gives `no_reading`. Any other row that does not bind gives `reading_mismatch`.
- Bridge reads no `usage_ledger` row for the decision.

### The due mark

- `handle_agent_value_timed` builds a receipt inside its lock from values that it holds already. The receipt adds no SQL.
- A completed turn gives a due mark when six conditions are true. The switch is on and the turn did not fail. `is_direct` is true and the depth is 0. The harness is allowed and the launch is known.
- These turns give no due mark: a failed turn, a provider error, a prompt-control turn and a checkpoint turn.
- The turn of an open attempt gives no due mark. A turn of an orchestrator or a worker gives no due mark.
- The turn end of a compaction gives no due mark. This holds for an early compaction, a manual `/compact`, and a compaction that the harness starts itself.
- The frame path sets one flag in memory for the chat when it handles a native boundary. The next completed turn of that chat clears the flag and gives no due mark.
- The flag does not use a turn id. Claude sends no turn start for a compaction, and its turn end has no turn id.
- The mark holds the turn id, the time of the turn end, the launch (`started_at` and `provider_session_id`) and the harness.
- After the lock, `early_compaction::accept` puts the mark in memory. A new mark of the same chat replaces the old mark.
- A fire removes the mark. A new mark needs a new completed turn that is not a compaction.

### One dispatch path

- `forward_native_compaction(core, session_id, harness, focus, origin)` holds the body of the manual native `/compact` of today. The slash command calls it with `CompactOrigin::Person`. The sweep calls it with `CompactOrigin::Early`.
- Both origins send the same adapter call. On Claude, the sweep sends a bare `/compact`. On Codex, the sweep sends `thread/compact/start` with the thread id only.
- The origin `Early` writes no focus message and never starts a Bridge checkpoint. If the result is `Unsupported`, the attempt settles as `unsupported`.
- The sweep calls the capability check of the registry before it takes any lock. So the schema probe of Codex never runs under the reservation.
- The sweep takes `input_activity.try_write()` and then `claim_session_lifecycle`. If it cannot take either one, the mark stays.
- With both held, the sweep reads the switch row, the opt-out row, the session row, the reading and the state. Then it decides and dispatches.
- Each user send holds `input_activity.read()` until it returns. So a user send and the early trigger cannot both dispatch.
- A save of the switch or of the opt-out holds `input_activity.read()` for its write. So a save and a dispatch do not interleave. An opt-out saved before the reservation wins.
- A user send after an early dispatch reads the status `working`. Bridge routes the send as it routes a send after a manual `/compact` today. The change adds no rule for routes.
- Stop, the queue drain at the boundary, and the refusal of images in the queue stay as today.

### Suppression state

- Approval: the status is `waiting`, or `attention_events` has a `needs_you` row for the chat with `resolved_at` null. The attention module closes these rows for each request id.
- The trigger resolves no approval and skips no approval. Policy stays the only resolver.
- Open tool: a `tool.started`, `command.started` or `file_change.started` entry after the last `turn.started` has no completion with the same `provider_event_id`. The scan reads at most `TOOL_SCAN_CAP` entries, newest first.
- If the scan finds no `turn.started` within the cap, the result is `state_unknown`.
- Children: the clauses of `runtime_budget::can_release`. They cover worker results with `result_status='pending'`, outbox rows of `worker.result` with `status='pending'`, live harness subagents, and background tasks with `in_progress=1`. One shared SQL fragment serves both callers.
- Queued input: a `queued_session_input` row in state `queued` or `claiming`.

### The attempt record

- Bridge creates the table `early_compaction_attempts` at store open with `CREATE TABLE IF NOT EXISTS`. There is no schema version change, so no backup copy of the store.
- Columns: `id`, `session_id`, `harness`, `model`, `launch_started_at`, `provider_session_id`, `trigger_turn_id`, `trigger_reading_id`, `trigger_used_tokens`, `trigger_window_tokens`, `threshold_tokens`, `reason`, `created_at`, `dispatched_at`, `outcome`, `boundary_pre_tokens`, `boundary_post_tokens`, `compaction_turn_id`, `next_reading_id`, `next_used_tokens`, `error`, `settled_at`.
- `compaction_turn_id` is the active turn id at the boundary. It is null on Claude.
- Two partial indexes come with it: open attempts by session, and open `needs_you` attention rows by session.
- Order of a fire: the sweep writes the row with a new id, adds the open entry in memory, dispatches, and then writes `dispatched_at`. The write of `dispatched_at` never changes `outcome`.
- `compacted`: a native boundary arrived while the attempt was open. Claude gives `preTokens` and `postTokens`. Codex gives no figures, so both columns stay null.
- The boundary settles the attempt, so `settled_at` is the time of the boundary. The turn end that follows the boundary changes nothing in the row.
- `ended_without_boundary`: the turn of the attempt completed with no boundary.
- `failed`: the turn of the attempt failed, or the adapter call returned an error. Bridge records the error and does not change the status.
- `unsupported`: `forward_native_compaction` returned `Unsupported`.
- `interrupted`: the launch changed, the status became `stopped`, `failed`, `completed` or `cancelled`, or Bridge restarted.
- `unsettled`: no terminal signal arrived within `SETTLE_LIMIT`. Bridge does not change the status.
- A boundary of another launch settles nothing. A later manual boundary settles nothing, because no open entry exists for it.
- Window before: the trigger reading. Window after: the next reading. This pair uses one base, so it is the pair that the audit compares.
- `boundary_pre_tokens` and `boundary_post_tokens` hold the report of Claude as given. `boundary_post_tokens` can be null. It uses another base than the readings, and a live probe showed it far below the next reading.
- The next reading is the first bound reading with `id` above `trigger_reading_id` and `created_at` at or after `settled_at`. It is a request reading, and it can belong to the next user turn.
- Claude: the next reading is the reading that the sidecar takes after the result of the compaction.
- Codex: the reading of the compaction turn arrives before the boundary and is not the window after. It is never the next reading, also when its time equals the time of the boundary. The next reading comes with the first request of the next turn.
- When the next reading of a `compacted` attempt is at or above `THRESHOLD_TOKENS`, the launch goes into the set that `launch_attempt_ineffective` reads.
- Settlement ignores the switch. A switch turned off does not stop settlement.
- At startup, before the maintenance loop, Bridge sets each open row to `interrupted`. It sends nothing. Resume stays with the startup path of today.

### The switch

- Configuration kind `early_compaction_settings`, id `global`, value `{"enabled": bool}`. Absent means off.
- Configuration kind `early_compaction_opt_out`, id is the chat id. A row means off for that chat.
- `config/get_early_compaction_settings {sessionId?}` returns `{enabled, globalEnabled, sessionOptedOut, sessionEligible}`. `sessionEligible` is true only for an existing direct chat with no parent, at depth 0, on an allowed harness. It is false when the request names no chat.
- `config/save_early_compaction_settings {sessionId?, enabled?}`: a global save needs `enabled`. A chat save of `false` writes the opt-out, and `null` deletes it. A chat save of `true` is `Invalid`.
- The host decides the scope with the checks of the rule: a chat save of `false` for a chat outside it is `Invalid` and writes nothing.
- Unknown fields are an error. The protocol minor is the next minor after T3. The generator makes the artifacts.
- A save sets the switch in memory after its write. At startup, Bridge reads the switch row into memory. The sweep reads the row again under the reservation.
- `settings_tools` has no entry for these kinds, so a chat cannot change them. A configuration reset deletes both kinds.
- The settings screen shows one switch in the `Coding agents` section. The chat menu shows the opt-out only when the global switch is on.

### Off guarantee

- Start state: the switch is absent or false, and no attempt row is open.
- For any sequence of sends, provider frames, sweeps and manual `/compact` commands, the change keeps four things equal to the base code.
- The four things: the ordered adapter calls with equal bytes, the ordered published `CoreEvent` values, the rows of each base table, and the transcript projection.
- The row comparison replaces timestamps and generated ids with fixed values.
- Allowed differences: the empty new table and its two indexes, reads of the switch, the protocol minor and the two new methods.
- If the switch goes off with an open attempt, settlement writes only that attempt row. After that, the guarantee holds.
- The golden trace `testing/fixtures/early-compaction/off-trace.json` comes from the base code, before any hook of this change.

### Bounded reads

- The frame path gains one map lookup. With an open attempt only, it also runs one UPDATE by primary key after the lock.
- The sweep reads in one hold of the database lock. Each statement uses a key, a partial index, or a cap.
- No code of this change walks parent pointers or calls `CompactionController::pending`.

### Weaknesses of today (PR notes, not new states)

- A Codex rejection of `thread/compact/start` leaves the chat `working` with no boundary. The request id is below `TURN_START_REQUEST_ID_BASE`, so the normalizer ignores the rejection.
- The manual `/compact` has this weakness today. Stop or the next message recovers the chat. The early attempt settles as `unsettled`, and `launch_attempt_failed` stops further attempts on that launch.
- A Claude write error leaves a stale turn id in the runtime. The status does not change, and the next send replaces the id. The manual `/compact` has the same behaviour.

## Unit Tests

`early_compaction::tests`, one test for each suppression:

- `suppresses_disabled`
- `suppresses_opted_out`
- `suppresses_harness_not_allowed_for_opencode`
- `suppresses_not_direct_for_an_orchestrator`
- `suppresses_not_direct_for_a_worker`
- `suppresses_not_direct_for_a_workflow_session`
- `suppresses_has_parent`
- `suppresses_unsupported`
- `suppresses_no_runtime`
- `suppresses_launch_mismatch_when_the_mark_has_no_launch`
- `suppresses_launch_mismatch_after_a_relaunch`
- `suppresses_launch_attempt_failed`
- `suppresses_launch_attempt_ineffective`
- `suppresses_attempt_open`
- `suppresses_too_soon`
- `suppresses_cache_cold`
- `suppresses_turn_active`
- `suppresses_approval_open`
- `suppresses_tool_open`
- `suppresses_children_live`
- `suppresses_queued_input`
- `suppresses_state_unknown`
- `suppresses_no_reading`
- `suppresses_reading_mismatch_for_another_model`
- `suppresses_reading_mismatch_for_another_thread`
- `suppresses_below_threshold`

`early_compaction::tests`, the rule, the reading and the mark:

- `fires_at_the_threshold`
- `suppressions_are_checked_in_the_stated_order`
- `keep_and_drop_follow_the_stated_list`
- `a_new_due_mark_replaces_the_old_one`
- `a_codex_reading_binds_by_turn_id`
- `a_claude_reading_binds_by_launch_and_time_after_the_turn_end`
- `a_claude_reading_from_before_the_turn_end_is_no_reading`
- `a_model_switch_in_the_idle_delay_is_a_reading_mismatch`

`early_compaction::tests`, the suppression state against a database:

- `an_open_approval_among_resolved_ones_suppresses` (requests A and B, B resolved)
- `a_write_scope_approval_suppresses_until_its_approval_resolves`
- `a_foreground_tool_with_no_completion_suppresses`
- `a_worker_result_or_outbox_delivery_not_done_suppresses`
- `a_tool_scan_past_the_cap_is_state_unknown`
- `a_failed_state_query_is_state_unknown`

`runtime_budget::tests`:

- `can_release_keeps_its_behaviour_with_the_shared_fragment`

`early_compaction_settings::tests`:

- `the_switch_is_off_when_absent`
- `the_global_switch_round_trips`
- `a_chat_can_opt_out_and_back`
- `a_chat_cannot_turn_the_feature_on`
- `a_global_save_without_a_value_is_refused`
- `a_save_sets_the_switch_in_memory`
- `a_configuration_reset_turns_it_off`
- `get_reports_the_scope_of_a_chat` (a missing id, a worker, a depth 1 direct chat, an OpenCode chat and eligible chats)
- `an_opt_out_for_a_chat_outside_the_scope_is_refused`

`live_turn::tests`, the due mark:

- `a_completed_direct_chat_turn_is_marked_due`
- `a_failed_turn_is_not_marked`
- `a_provider_error_is_not_marked`
- `a_prompt_control_turn_is_not_marked`
- `a_checkpoint_turn_is_not_marked`
- `the_turn_of_an_open_attempt_is_not_marked`
- `the_turn_end_of_a_compaction_makes_no_due_mark` (the recorded frame order of Claude and of Codex, for an early compaction, a manual `/compact` and a compaction that the harness starts)
- `the_turn_after_a_compaction_is_marked_again`
- `a_fire_removes_the_due_mark`
- `an_orchestrator_or_worker_turn_is_not_marked`
- `no_mark_with_the_switch_off`
- `with_the_switch_on_a_turn_frame_adds_no_sql`

`live_turn::tests`, the single dispatch path:

- `the_slash_command_and_the_early_trigger_share_forward_native_compaction`
- `a_user_send_and_the_early_trigger_cannot_both_dispatch` (both orders)
- `a_send_after_an_early_dispatch_routes_as_after_a_manual_compact` (legacy send, composer default, queue preference, steer preference, image)
- `an_opt_out_saved_before_the_reservation_wins`
- `a_switch_turned_off_before_the_reservation_wins`
- `a_save_waits_for_a_dispatch_in_flight`
- `a_taken_lifecycle_claim_keeps_the_mark`
- `idle_release_and_the_early_trigger_cannot_both_act`
- `an_early_compaction_never_starts_a_bridge_checkpoint`
- `stop_during_an_early_compaction_settles_the_attempt_as_interrupted`

`live_turn::tests`, the manual `/compact` tests keep their assertions with no change:

- `a_focus_accepting_harness_gets_the_focus_and_bridge_stays_out_of_it`
- `a_whole_conversation_harness_says_the_focus_was_not_applied`
- `a_bare_compact_on_a_whole_conversation_harness_explains_nothing`
- `a_harness_with_no_compaction_command_falls_back_to_a_bridge_checkpoint`
- `a_forwarded_compaction_marks_the_turn_it_started`
- `a_forwarded_compaction_is_not_a_bridge_maintenance_turn`
- `a_boundary_releases_the_optimistic_mark_a_forwarded_compact_wrote`

The adapter command for each harness:

- `live_turn::tests::an_early_fire_on_claude_sends_a_bare_compact` (the runtime records focus `None`)
- `claude_adapter::tests::compact_forwards_the_focus_on_the_input_stream` (no change, `compact_command(None)` is `/compact`)
- `live_turn::tests::an_early_fire_on_codex_sends_thread_compact_start`
- `codex_adapter::tests::compact_start_request_carries_the_thread_id_and_nothing_else` (no change)
- `live_turn::tests::an_opencode_chat_is_never_fired`

`live_turn::tests`, the attempt record:

- `an_attempt_row_exists_before_the_adapter_call`
- `a_claude_boundary_settles_the_attempt_with_pre_and_post_tokens`
- `a_claude_boundary_without_post_tokens_settles_with_a_null_after_figure`
- `a_codex_boundary_settles_with_absent_figures_and_labeled_readings`
- `a_boundary_before_the_dispatched_write_settles_its_attempt`
- `a_later_manual_boundary_is_not_credited_to_an_early_attempt`
- `a_boundary_of_another_launch_settles_nothing`
- `a_turn_end_without_a_boundary_settles_as_ended_without_boundary`
- `an_adapter_error_settles_as_failed_and_leaves_the_status`
- `no_terminal_signal_within_the_limit_settles_as_unsettled`
- `a_launch_change_settles_the_open_attempt_as_interrupted`
- `an_unsettled_attempt_stops_further_attempts_on_that_launch`
- `the_next_reading_fills_after_a_compacted_attempt`
- `a_claude_reading_after_the_compaction_result_is_the_next_reading`
- `a_codex_reading_of_the_compaction_turn_is_not_the_next_reading` (also with a time equal to the time of the boundary)
- `a_next_reading_at_the_threshold_stops_further_attempts_on_that_launch`
- `a_next_reading_below_the_threshold_lets_a_later_turn_fire`
- `the_turn_end_after_a_boundary_does_not_change_the_settled_row`
- `disable_does_not_skip_settlement`
- `an_open_attempt_is_settled_as_interrupted_at_startup_with_no_second_send`

`live_turn::tests`, the off guarantee:

- `with_the_switch_off_send_frame_and_manual_compaction_traces_are_unchanged` (switch absent, and switch saved as false, against the golden trace)
- `with_the_switch_off_the_sweep_runs_no_sql`
- `with_the_switch_off_the_attempt_table_stays_empty`

`bridge_protocol`:

- `handshake::tests::early_compaction_client_rejects_a_daemon_without_it` (an old daemon refuses a new client, a new daemon accepts an old client)
- `messages::config::tests::early_compaction_settings_params_refuse_unknown_fields`
- `tsgen::tests::checked_in_artifacts_match_the_contract` passes after the generator runs

Frontend:

- `src/components/settings/EarlyCompactionSetting.test.tsx`: the switch reads and saves the global value in mock mode.
- `src/components/BridgeSidebar.test.tsx`: the chat menu shows the opt-out only when the global switch is on.
- `src/components/BridgeSidebar.test.tsx`: `the host decides which chat gets the opt-out` and `the mock reports the scope of each chat of its data`.

## Integration / Functional Tests

- `early_compaction_eval::offline`, always in `cargo test`. It tests only the rule and the suppressions.
- Input: the synthetic cases of `testing/evals/early-compaction/rule-cases.json`. Each case is a sequence of receipts, readings, state snapshots and clock steps, with the expected decision for each sweep.
- The test runs the real `rule` and the real `Tracker`. It makes no claim about tokens.

## Smoke Tests

- `bun run build`
- `bun run test`
- `node scripts/clippy-ratchet.mjs`, with no new `unwrap`, `expect` or `panic`, test code too
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-protocol`

## E2E Tests

`early_compaction_eval::live`, with `#[ignore]`. It asserts that `BRIDGE_EARLY_COMPACTION_EVAL` is set.

- `BRIDGE_EARLY_COMPACTION_EVAL_HARNESS`: `claude`, `codex`, or both when absent.
- `BRIDGE_EARLY_COMPACTION_EVAL_MODEL`: the default for Claude is `claude-haiku-5-5`, the cheapest current Claude model. The default for Codex is `gpt-6.2-sol`.
- `BRIDGE_EARLY_COMPACTION_EVAL_ONLY`: one harness or one arm.
- The workload `testing/evals/early-compaction/workload.json` has a fixture repository and 14 scripted turns.
- Turns 1 to 3 state decisions in the conversation only. Turns 4 to 10 read and edit files until the window passes `THRESHOLD_TOKENS`. Turns 11 to 14 need the decisions of turns 1 to 3.
- The eval runs the workload two times. Arm `on` saves the switch through `early_compaction_settings::save`. Arm `off` leaves the switch absent.
- Both arms start `start_queued_input_maintenance`, the loop that the daemon starts. Both arms use the shipped constants.
- After each turn, both arms wait until the due mark has a final decision, or until `WARM_LIMIT`.
- Rubric for turns 11 to 14: R1 gives 1 point when the check command passes. R2 gives 1 point when the files keep each decision. The maximum is 8 points for each arm.
- R1 runs the `check.mjs` text of the workload from a new temporary folder, not the copy in the chat directory. The working directory is the chat directory.
- Before each score, the eval compares `src/money.js` and `src/errors.js` with the workload. A changed or missing helper fails the arm with `fixture_changed`.
- Unit tests in `early_compaction_eval::driver::tests`: `a_checker_that_the_chat_disabled_does_not_pass_a_wrong_module`, `a_changed_helper_fails_the_arm_with_a_named_reason`.
- The check of each function has two or more half-even ties, where `Math.round` and `toFixed` give a different answer. It also needs a `LedgerError` with an UPPER_SNAKE `code` for each error case.
- R2 reads the file with no comments, so a rule only in a comment gives no point. Only the header rule reads its comment line. R2 also refuses `Math["round"]`, `Math['round']` and `["toFixed"]`.
- Unit tests in `early_compaction_eval::workload::tests`: `a_rule_only_in_a_comment_gives_no_point`, `a_forbidden_rounding_by_its_quoted_name_gives_no_point`.
- The eval reads `usage_ledger` for the whole chat, with the compaction turn. It prints uncached input, cache write, cache read and output for each arm.
- The eval reads the ledger sums after the last scripted turn and before the session stops. A stop runs one Bridge checkpoint turn with its own ledger rows.
- Claude: the ledger row of a compaction request holds the real share of that request. The eval sums the rows as they are.
- Codex: the harness does not report the tokens of a compaction request. For each sent attempt (a dispatch time, or the outcome `compacted`), the eval adds `trigger_used_tokens` one time as uncached input to arm `on`.
- The report prints that Codex figure in its own column with the label `compaction estimate (not observed)`. It is not a bound: Codex can send one compaction request again.
- It also prints each attempt row: outcome, trigger reading, `boundary_pre_tokens`, `boundary_post_tokens` and the next reading.
- Pass bar for each harness, all required:
  - Both arms finish every turn.
  - Arm `on` has at least one attempt with outcome `compacted`. On Claude, that attempt has `boundary_pre_tokens`.
  - Claude: the ledger has a row with tokens above zero for each dispatched compaction request, with any outcome. If not, the harness fails with `compaction_usage_unobserved`.
  - Codex: each sent attempt has a trigger reading for the estimate. If not, the harness fails with `compaction_usage_unobserved`.
  - The score of arm `on` is at least the score of arm `off` minus 1. The score of arm `off` is at least 6.
  - The observed sum of uncached input, cache write and cache read of arm `on` is below the same sum of arm `off`.
- Codex: when arm `on` has a compaction estimate and each part of the bar holds, the verdict is `INCONCLUSIVE`, not `PASS`. The eval cannot observe the usage of a compaction request. An `INCONCLUSIVE` verdict does not fail the test.
- Unit tests in `early_compaction_eval::report::tests`: `the_codex_estimate_adds_each_sent_trigger_one_time`, `the_codex_bar_counts_the_estimate_and_needs_a_trigger_reading`, `a_codex_comparison_with_unobserved_compaction_requests_does_not_pass`.
- The eval prints a price estimate from `usage_pricing` as a labeled side figure. A model with no rate prints `no rate`. The price is not part of the pass bar.
- The switch stays off by default until both harnesses pass. The PR shows the output.

## Manual Tests

- Run `bun run tauri dev`. Turn the switch on. Grow a Claude chat past `THRESHOLD_TOKENS`. After `IDLE_DELAY`, the status row shows the compaction, and the boundary card of today appears.
- Do the same on Codex with `gpt-6.2-sol`.
- Send a message while the compaction runs. The message stops the compaction and goes out next, as after a manual `/compact`.
- Turn the chat opt-out on. Grow the same chat. No compaction starts.
- Turn the global switch off. The chat menu hides the opt-out.

## Out of Scope

- Orchestrators, workers, workflow sessions and sessions with a parent.
- OpenCode.
- A focus text on any harness.
- A second threshold, a phase signal, or a model choice of the moment.
- A new label on the boundary card.
- A request id band for Codex compaction requests.
- A change to the content of a Bridge checkpoint.
- A restart of a harness process, or a replay of a shorter history.
