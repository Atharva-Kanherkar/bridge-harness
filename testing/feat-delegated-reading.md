# `feat/delegated-reading`: Test Contract

This PR measures how much of the main window comes from file reads and searches. It changes nothing that a model sees. It also makes the generic chat launchers refuse a Bridge worker. The prompt rule for read-only workers (step 2) is not in this PR. No wire change, no schema change, no migration, no generated artifact, and no new work on the per-frame path.

## Functional Behavior

### The token report (`bridge-core/src/token_report.rs`)

- The report opens the database with `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX` and `PRAGMA query_only=ON`. It never calls `store::open`.
- All queries of one report run in one read transaction. A write from another connection after the first read does not change the result.
- Every value in a query is a bind parameter. An optional column name comes from a fixed list of literals in the module.
- A schema version above `LATEST_SCHEMA_VERSION` gives a plain error. The report does not read further.
- An absent required column gives a plain error. The text names the column. It tells the owner to open the file one time with a current Bridge build, then take a new backup.
- An absent optional column gives an exact fallback, and the output header names it:
  - no `fork_parent_session_id` or `fork_parent_entry_id`: no session is a fork
  - no `context_usage_after_id`: the value is 0
  - no `usage_ledger.context_used_tokens`: `lastContextUsed` and `maxContextUsed` are null
- A NULL `token_estimate` falls back to `entry_tokens::estimate(kind, payload)`.
- The report reads only the kinds in `entry_tokens::ESTIMATED_KINDS`.

### Items and the key of an item

- The key of a row is `Call(payload.itemId)`. Without that field, the key is `Event(provider_event_id)`. Without both, the key is `Entry(id)`.
- Keys of two sources never merge. Keys of two sessions never merge.
- Rows group by kind and key. A group counts one time, with the largest estimate of its rows.
- A group has no estimate when no row gives one and one row or more is not a Claude start with `phase` equal to `preparing`. Each such group adds 1 to `unestimated`.

### Classes

- Classes: `read`, `search`, `shell`, `edit`, `web`, `mcp`, `agent`, `other_tool`, `conversation`, `bridge`.
- `user.message`, `assistant.message`, `reasoning` and `reasoning.completed` are `conversation`.
- `session.context`, `delegation.result`, `context.compacted`, `checkpoint`, `compaction`, `branch.summary`, `handoff.brief` and `worker.result` are `bridge`.
- `file_change.*` is `edit`.
- A tool group takes its class from the first source that names a tool, in this order:
  1. a start row with the same key in the same session
  2. the fields of its own completion row
  3. else `other_tool`
- The fields, in order: Codex `data.type`, OpenCode `data.tool`, Claude and import `data.name`, Claude `data.command` on a Bash completion. The report never reads `title`.
- `web` is its own class. It is not part of `readingShare`.
- `readingShare` is `read` plus `search`, divided by `total`. `shellShare` is `shell` divided by `total`. Both are 0 when `total` is 0.

### The shell rule

- The rule unwraps `sh`, `bash` or `zsh` (with a path or without one) with `-c`, `-lc` or `-cl` one time. It accepts a string or an argv array.
- A command text with any of these characters is `shell`: `|`, `&`, `;`, `<`, `>`, `(`, `)`, `$`, a backtick, a newline. Quotes do not change this.
- A text that `shell_words::split` cannot split is `shell`. An empty command is `shell`. A first word of the form `NAME=value` is `shell`.
- Read programs:
  - `cat`, `head`, `tail`, `nl`, `wc`
  - `sed -n` with one script of the form `<N>p` or `<N>,<M>p`, plus paths
  - `git show`, `git diff`, `git log`, `git blame`, with no argument that starts with `--output`
- Search programs:
  - `rg` with no argument that starts with `--pre`
  - `grep`, `ls`
  - `fd` with no `-x`, `-X`, `--exec` or `--exec-batch`
  - `find` with no `-exec`, `-execdir`, `-ok`, `-okdir`, `-delete`, `-fprint`, `-fprintf` or `-fls`
  - `git grep`, `git ls-files`
- Any other program is `shell`.

### Cohort, recency and forks

- Cohort `orchestrator`: `kind = 'orchestrator'`, a workspace, no parent, depth 0, and no `worker_runtime` row.
- Cohort `direct`: `kind = 'direct'`, no parent, depth 0, and no `worker_runtime` row.
- No other kind is in a cohort. This excludes `imported`, `workflow`, `harness_subagent` and the hidden helper kinds.
- A fork copies the branch of its source up to the fork point as its first K entries. K is the length of the parent chain of `fork_parent_entry_id` in the source.
- The first K entries of a fork belong to the source. They count in neither the totals nor the activity of the fork.
- If the source entry is absent, K is 0 and `forkPrefixUnknown` is true.
- Activity is an own `user.message` or `assistant.message`. A session is recent when its last activity, by sequence, is at or after `since` at 00:00 UTC.
- The report parses the timestamp as RFC 3339. A value that does not parse is not recent, and it adds 1 to `unreadableTimestamps`.
- `since` is 30 days before the current UTC date. `--since` changes it.
- `--session` reports one session and skips recency. A session outside both cohorts gives a plain error.

### Rank and output

- The report computes the exact total of every eligible session. It sorts by total, largest first, then by session id, smallest first. It keeps the first `top` rows. `eligible` is the count before the cut.
- A recent session with a total of 0 stays in the list.
- `lastContextUsed` and `maxContextUsed` come from ledger rows with `id` above `context_usage_after_id`, in the order of `id`.
- The markdown output has a header line, one line for each fallback, and one table. The JSON output is the `Report` object.
- The output contains ids, harness names, kinds, numbers and timestamps. It never contains a label, a title, chat text, tool input or tool output.

### The command (`bridge tokens`)

- Flags: `--database <path>`, `--data-dir <dir>`, `--cohort orchestrator|direct`, `--since YYYY-MM-DD`, `--top <n>`, `--session <id>`, `--json`.
- Without a database flag, the path is `default_data_dir()` plus `bridge.db`.
- Defaults: cohort `orchestrator`, top 10, since 30 days before today.
- Exit 2 with the usage text: an unknown flag, an unknown cohort, a bad date, `--top 0`, or both database flags.
- Exit 1 with the error text: a report error.

### Worker start guards

- `start_chat_claimed` refuses a session that is not a top-level chat (`chat_defaults::is_top_level_chat`). The check runs before any lock of the workspace, any process stop and any UPDATE.
- The refusal uses the text of `deliver_queued_now`, from one shared constant.
- After a refusal, the `depth`, `parent_session_id` and `kind` of the worker do not change. A live process of the worker stays in place.
- `start_session` reuses only a session with no parent and no `worker_runtime` row. Without such a session, it makes a new one, as it does now.
- The Claude read-only hook denies `Agent` and `Task`, with network access and without it. The `tools` list never contains `Agent`. This PR adds a test and no code.

### Decision rule (locked before anyone runs the report on the owner database)

- Step 2 can start only when 6 or more of the 10 largest eligible orchestrator chats have a `readingShare` of 0.30 or more.
- If fewer than 10 orchestrator chats are eligible, the result is inconclusive, and step 2 does not start.
- Step 2 also needs evidence for each prerequisite (a) to (e) in the plan note.
- The written decision gives the table, the count of chats at 0.30 or more, and the `shellShare` of each row. The shell share has no threshold.
- Nobody changes these values after the owner runs the report. A change needs a new contract and a new run.

## Unit Tests

### `token_report::tests`

Classes:

- `claude_tool_names_map_to_classes`
- `opencode_part_tools_map_to_classes`
- `codex_item_types_map_to_classes`
- `text_kinds_are_conversation_and_injected_kinds_are_bridge`
- `web_tools_are_web_and_outside_the_reading_share`
- `title_never_decides_a_class`

Keys and groups:

- `a_completion_takes_the_class_of_its_start_by_payload_item_id`
- `a_forked_start_and_completion_pair_without_provider_event_ids`
- `an_imported_completion_pairs_by_payload_item_id_not_source_id`
- `fallback_keys_are_scoped_by_session_and_source`
- `each_item_counts_once_per_kind_with_its_largest_estimate`
- `a_null_estimate_falls_back_to_entry_tokens`
- `a_preparing_start_with_a_full_start_is_estimated`
- `an_item_with_no_estimate_counts_once_as_unestimated`

Completion-only records:

- `a_completion_only_opencode_part_takes_its_class_from_its_tool`
- `a_completion_only_codex_item_takes_its_class_from_its_type`
- `a_completion_only_import_takes_its_class_from_data_name`
- `a_completion_only_claude_bash_result_uses_its_command`
- `a_completion_with_no_start_and_no_tool_fields_is_other_tool`

Shell rule:

- `simple_reads_are_read`
- `simple_searches_are_search`
- `known_wrappers_are_unwrapped_once`
- `argv_commands_follow_the_same_rule`
- `any_shell_operator_is_shell_even_in_quotes`
- `writes_and_exec_flags_are_shell`
- `unknown_programs_and_unsplittable_commands_are_shell`

Cohort and recency:

- `the_orchestrator_cohort_keeps_top_level_workspace_orchestrators_only`
- `the_direct_cohort_keeps_top_level_direct_chats_only`
- `imported_workflow_and_hidden_sessions_are_in_no_cohort`
- `recency_uses_the_last_own_message_not_the_last_entry`
- `since_changes_the_window`
- `an_unreadable_timestamp_is_not_recent_and_is_counted`
- `the_session_option_rejects_a_worker`

Forks:

- `a_fork_excludes_its_copied_prefix_from_tokens_and_activity`
- `a_fork_of_a_fork_excludes_only_its_own_prefix`
- `a_fork_with_an_absent_source_entry_counts_every_entry_and_says_so`

Rank:

- `the_top_ten_comes_from_exact_totals_of_every_eligible_session`
- `a_rank_tie_breaks_by_session_id`
- `an_empty_recent_session_ranks_last_with_zero_shares`

Schema and the old database:

- `a_schema_59_database_reads_with_exact_fallbacks`
- `a_database_without_a_required_column_is_a_plain_error`
- `a_newer_schema_version_is_a_plain_error`

Transaction and writes:

- `the_report_connection_rejects_writes`
- `one_read_transaction_hides_a_concurrent_write`

Window columns:

- `window_columns_use_the_current_provider_epoch`
- `window_columns_are_null_without_the_ledger_column`

Output:

- `markdown_output_matches_a_fixed_table`
- `json_output_has_the_documented_fields`
- `no_output_contains_payload_text`

### `entry_tokens::tests`

- `estimated_kinds_match_the_estimator`
- All current tests stay green, with `a_preparing_tool_start_is_not_counted`.

### `bridge` binary (`bridge-client/src/bin/bridge.rs`)

- `tests::tokens_flags_have_defaults`
- `tests::tokens_flags_accept_every_option`
- `tests::tokens_rejects_an_unknown_cohort`
- `tests::tokens_rejects_a_bad_since_date`
- `tests::tokens_rejects_two_database_flags`
- `tests::tokens_prints_markdown_for_a_seeded_database`
- `tests::tokens_prints_json_for_a_seeded_database`

### Closures that ship

- `live_turn::tests::start_chat_refuses_a_bridge_worker_and_launches_nothing`
- `live_turn::tests::start_chat_refuses_a_live_worker_without_replacing_its_process`
- `live_turn::tests::start_chat_still_reaches_the_launcher_for_a_top_level_chat`
- `live_turn::tests::start_session_reuse_skips_workers_and_harness_subagents`
- `policy_coordinator::tests::a_depth_one_session_cannot_delegate_reading`
- `sidecar/claude-agent/test/read-only.mjs`: `a read-only worker cannot start a Claude subagent`
- Current tests that stay green: `live_turn::tests::deliver_now_refuses_a_worker_and_launches_nothing`, `live_turn::tests::deliver_now_resumes_an_adapterless_chat`, `live_turn::tests::harness_child_stop_and_parent_stop_keep_siblings_and_the_shared_runtime`, and the sidecar test `the Bridge hook allows local reads and explicitly denies mutation`.

## Integration / Functional Tests

- The seeded tests use a `NamedTempFile` database that `store::open` makes. It holds one chat of each harness shape, one worker, one harness subagent, one import, one workflow session, one hidden session and one fork.
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core token_report::` is green.
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core live_turn::tests::start_` is green.
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core policy_coordinator::` is green.
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-client` is green.
- `node --test sidecar/claude-agent/test/read-only.mjs` is green.

## Smoke Tests

- `bun run check` and `bun run build` are green.
- `bun run test` is green, or the PR shows each failure as unrelated, with output.
- `bridge tokens --database <copy> --since 2026-09-01` on a backup of the local database prints a table and exits 0. The size and the SHA-256 of the copy do not change.

## E2E Tests

- None in this PR. The eval belongs to step 2 and to prerequisite (e).

## Manual Tests

- Take a backup with `sqlite3 <live> ".backup '<copy>'"`. Run `bridge tokens --database <copy> --json` on it. Put the table, the count of chats at 0.30 or more, and the decision in the PR.
- Run the same command with `--cohort direct`. Put that table in the PR as information only.
- In the app, open a stopped worker and click the compaction retry if the card shows. The app shows the refusal, and the worker stays stopped.

## Out of Scope

- The prompt rule for read-only workers, its eval and its policy tests (step 2).
- The Codex `features.multi_agent` override and the OpenCode `task` deny (prerequisite d).
- A change to the result notice of a worker (prerequisite a) or to the evidence load (prerequisite b).
- A backfill of `token_estimate`, and a weight for the number of times a harness reads an entry again.
- Codex `commandActions` as a source of a class.
- The reuse of `imported`, `workflow` and hidden sessions by `start_session`.
