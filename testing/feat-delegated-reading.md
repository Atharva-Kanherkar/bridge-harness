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
- An own `session.context` row with `itemId` equal to `orchestrator-briefing` uses `Entry(id)`. Each such row contributes one recorded launch estimate.
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
- `shellRead` counts the estimated tokens of `shell` groups that pass the read-only shell rule. It is a subset of `shell`.
- `combinedShare` is `read` plus `search` plus `shellRead`, divided by `total`. It is 0 when `total` is 0.

### The shell rule

- Both shell rules accept a bare program name or an absolute program path directly in `/bin`, `/usr/bin`, `/usr/local/bin` or `/opt/homebrew/bin`. This applies to sources, filters and wrappers. A relative program path or any other absolute program path gives `shell` with no `shellRead` credit.
- The strict rule unwraps `sh`, `bash` or `zsh` with `-c`, `-lc` or `-cl` one time. It accepts a string or an argv array.
- A command text with any of these characters is `shell`: `|`, `&`, `;`, `<`, `>`, `(`, `)`, `$`, a backtick, a newline. Quotes do not change this.
- In argv without a wrapper, the same character check applies to each word.
- A text that `shell_words::split` cannot split is `shell`. An empty command is `shell`. A first word of the form `NAME=value` or `NAME+=value` is `shell`.
- Read programs:
  - `cat`, `head`, `tail`, `nl`, `wc`
  - `sed -n` with one script of the form `<N>p` or `<N>,<M>p`, plus paths
  - `git show`, `git diff`, `git log`, `git blame`, with no argument that starts with `--output`
  - `git show`, `git diff` and `git log` also reject `--ext-diff` and `--textconv`.
- Search programs:
  - `rg` with no argument that starts with `--pre`
  - `grep`, `ls`
  - `fd` with no `-x`, `-X`, `--exec` or `--exec-batch`
  - `find` with no `-exec`, `-execdir`, `-ok`, `-okdir`, `-delete`, `-fprint`, `-fprintf` or `-fls`
  - `git grep`, `git ls-files`. `git grep` rejects a short option with `O` and `--open-files-in-pager`, with or without a value.
- Any other program is `shell`.

### The read-only shell rule

```text
command = ["cd" directory "&&"] pipeline {("&&" | ";") pipeline}
pipeline = source {"|" filter}
```

- Apply this rule only to a group in `shell`. Use the command from the same row that supplies the class.
- Split at `&&`, `;` and `|` only outside quotes. Each segment must pass `shell_words::split`. A `cd` directory must be literal and nonempty. It cannot start with a hyphen or have the form `+N`, where `N` contains only digits.
- Unwrap one `sh`, `bash` or `zsh` command with exactly three words and `-c`, `-lc` or `-cl`. The rule accepts text or argv. Other argv uses the strict operator check and stays one literal command.
- Reject empty stages, final separators, assignments (`NAME=value` and `NAME+=value`), nested wrappers, `||` and a background `&`. Reject redirects, dollar signs, backticks, parentheses, backslashes, carriage returns, newlines and NUL.
- Reject unquoted glob, brace, tilde, comment and history syntax. Reject unknown programs and options.
- Sources are `cat`, `head`, `tail`, `nl`, `wc`, numeric-print `sed`, `rg`, `grep`, `ls`, `fd`, `find` and the Git forms below. The rule checks flags and literal arguments for each program.
- The first five sources accept file paths and display or count flags. Search sources accept ordinary search flags, such as `grep -rn` or `fd -e EXT`.
- A `head` or `tail` operand cannot start with `+`.
- `rg` needs a pattern unless it has `--files`. With `--files`, it accepts optional paths and no pattern.
- `rg` accepts these options:
  - `-g` or `--glob`, `--iglob`, `-t` or `--type`, and `-T` or `--type-not`, each with a value
  - `-i`, `-S`, `-s`, `-F`, `-w`, `-x`, `-n`, `-N`, `-l`, `-c`, `-o`, `-v`, `-H`, `--no-filename`
  - `-A`, `-B`, `-C`, `-m` or `--max-count`, and `--max-depth`, each with a number
  - `-e` with a pattern
  - `--hidden`, `--no-ignore`, `-u`, `-uu`, `--no-heading`, `--heading`, `--json`, `--column`, `--trim`
  - `--color never` and `--sort path`
- An `rg` long option that takes a value also accepts `--flag=value`. Reject any `rg` option outside the list, such as `-z`, `--search-zip` and `--pre*`.
- `find` accepts paths and only `-name`, `-iname`, `-path`, `-type`, `-mindepth`, `-maxdepth` and `-print`. Reject `!` in the path list.
- Git forms are `grep`, `ls-files`, `show REV:path`, `diff --no-ext-diff --no-textconv`, `log` and `blame --no-textconv`. The rule accepts literal operands and a fixed set of options for each form.
- Reject `tee`, `xargs`, `awk`, `sed -i` and any other `sed` script. Reject the strict rule's `fd` and `find` actions, Git configuration overrides, `git grep -O`, `--open-files-in-pager` and `--output*`.
- The pipe filters are `head`, `tail`, `wc`, `sort`, `uniq`, `cut`, `nl` and `tr`. They accept specific display, comparison, count or selection flags and no file operands.
- Reject `sort -o`, `sort --compress-program` and any positional `uniq` output file. If any stage fails, the group stays in `shell` and adds 0 to `shellRead`.
- The rule checks command text only. Executable behavior and local configuration can add effects that the report does not see. Examples include `RIPGREP_CONFIG_PATH`, shell hooks, Git configuration, a pager and a Git index stat refresh.

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
- `--session` reports one session, infers its cohort and skips recency. It conflicts with an explicit `--cohort`. A session outside both cohorts gives a plain error.

### Rank and output

- The report computes the exact total of every eligible session. It sorts by total, largest first, then by session id, smallest first. It keeps the first `top` rows. `eligible` is the count before the cut.
- A recent session with a total of 0 stays in the list.
- `lastContextUsed` and `maxContextUsed` come from ledger rows with `id` above `context_usage_after_id`, in the order of `id`.
- The markdown output has a header line, one line for each fallback, and one table. The JSON output is the `Report` object.
- The Markdown table and JSON include `shellRead` and show `combinedShare` next to `readingShare`.
- The output contains ids, harness names, kinds, numbers and timestamps. It never contains a label, a title, chat text, tool input or tool output.

### The command (`bridge tokens`)

- Flags: `--database <path>`, `--data-dir <dir>`, `--cohort orchestrator|direct`, `--since YYYY-MM-DD`, `--top <n>`, `--session <id>`, `--json`.
- Without a database flag, the path is `default_data_dir()` plus `bridge.db`. The command prints the backup note with that path on stderr.
- Defaults: cohort `orchestrator`, top 10, since 30 days before today.
- Each flag can appear one time. The two database flags conflict. `--cohort` and `--session` conflict.
- An unknown flag, an unknown cohort, a bad date, `--top 0`, a repeat or a conflict causes exit 2 with the usage text.
- An empty value for `--database`, `--data-dir`, `--session`, `--cohort`, `--since` or `--top` also causes exit 2 with the usage text.
- Exit 1 with the error text: a report error.

### Worker start guards

- `start_chat_claimed` refuses a session that is not a top-level chat (`chat_defaults::is_top_level_chat`). The check runs before any lock of the workspace, any process stop and any UPDATE.
- The refusal uses the text of `deliver_queued_now`, from one shared constant.
- After a refusal, the `depth`, `parent_session_id` and `kind` of the worker do not change. A live process of the worker stays in place.
- `start_session` can reuse a `direct` or `orchestrator` session of the requested workspace and harness. The session must have a reusable status, no parent, no `worker_runtime` row and no pending native fork. If no session qualifies, it makes a new orchestrator session.
- The Claude read-only hook denies `Agent` and `Task`, with network access and without it. The `tools` list never contains `Agent`. This PR adds a test and no code.

### Decision rule (locked before anyone runs the report on the owner database)

- Step 2 can start only when 6 or more of the 10 largest eligible orchestrator chats have a `combinedShare` of 0.30 or more.
- If fewer than 10 orchestrator chats are eligible, the result is inconclusive, and step 2 does not start.
- Step 2 also needs evidence for each prerequisite (a) to (e) in the plan note.
- The written decision gives `readingShare`, `combinedShare` and `shellShare` for each row. It gives the count of chats with an unrounded `combinedShare` of 0.30 or more.
- The owner runs the report again under this contract. A later change to the rule needs a new contract and a new owner run.

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
- `each_recorded_orchestrator_briefing_counts_once_per_entry`
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
- `an_argv_with_an_operator_character_is_shell`
- `writes_and_exec_flags_are_shell`
- `unknown_programs_and_unsplittable_commands_are_shell`
- `shell_read_recognizes_read_only_compounds`
- `shell_read_rejects_unsafe_or_unknown_segments`
- `shell_read_rejects_invalid_command_forms`
- `shell_read_is_counted_once_with_the_selected_group_class`

Cohort and recency:

- `the_orchestrator_cohort_keeps_top_level_workspace_orchestrators_only`
- `the_direct_cohort_keeps_top_level_direct_chats_only`
- `imported_workflow_and_hidden_sessions_are_in_no_cohort`
- `recency_uses_the_last_own_message_not_the_last_entry`
- `since_changes_the_window`
- `an_unreadable_timestamp_is_not_recent_and_is_counted`
- `a_single_session_report_still_counts_an_unreadable_timestamp`
- `the_session_option_rejects_a_worker`

Forks:

- `a_fork_excludes_its_copied_prefix_from_tokens_and_activity` uses `fork_session_records` on a branched source. It checks contiguous copy sequences, token exclusion and recency.
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

- `markdown_output_matches_a_fixed_table` checks the `shellRead` and `combinedShare` columns.
- `json_output_has_the_documented_fields` checks the `shellRead` and `combinedShare` fields.
- `no_output_contains_payload_text`

### `entry_tokens::tests`

- `estimated_kinds_match_the_estimator`
- All current tests stay green, with `a_preparing_tool_start_is_not_counted`.

### `bridge` binary (`bridge-client/src/bin/bridge.rs`)

- `tests::tokens_flags_have_defaults`
- `tests::tokens_flags_accept_cohort_options`
- `tests::tokens_flags_accept_session_options`
- `tests::tokens_rejects_an_unknown_cohort`
- `tests::tokens_rejects_a_bad_since_date`
- `tests::tokens_rejects_two_database_flags`
- `tests::tokens_rejects_repeated_flags`
- `tests::tokens_rejects_cohort_with_session`
- `tests::tokens_prints_markdown_for_a_seeded_database`
- `tests::tokens_prints_json_for_a_seeded_database`
- `bridge-client/tests/client.rs`: `tokens_warns_on_stderr_only_for_the_default_database`

### Closures that ship

- `live_turn::tests::start_chat_refuses_a_bridge_worker_and_launches_nothing`
- `live_turn::tests::start_chat_refuses_a_live_worker_without_replacing_its_process`
- `live_turn::tests::start_chat_still_reaches_the_launcher_for_a_top_level_chat`
- `live_turn::tests::start_session_reuse_skips_workers_and_harness_subagents`
- `live_turn::tests::start_session_reuses_only_direct_and_orchestrator_chats`
- `policy_coordinator::tests::a_depth_one_session_cannot_delegate_reading`
- `sidecar/claude-agent/test/read-only.mjs`: `a read-only worker cannot start a Claude subagent`
- `live_turn::submit_input_tests::deliver_now_refuses_a_worker_and_launches_nothing`
- `live_turn::submit_input_tests::deliver_now_resumes_an_adapterless_chat`
- `live_turn::permission_policy_tests::harness_child_stop_and_parent_stop_keep_siblings_and_the_shared_runtime`
- `sidecar/claude-agent/test/read-only.mjs`, `the Bridge hook allows local reads and explicitly denies mutation`

## Integration / Functional Tests

- Seeded report and CLI tests open separate migrated databases with `store::open` in temporary directories. Each case seeds the rows it needs. The cases cover harness shapes, workers, harness subagents, imports, workflow and hidden sessions, and forks.
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

- Run `bridge tokens --database <copy> --json` on a backup available to the author. Record the commit, command, schema, eligible and shown counts, exit status and database checksums before and after the run in the PR. The checksums must match.
- If the author has no access to the decision database, record the decision as pending. The database owner runs the locked report on their backup. The owner records the top ten rows, the count at the threshold and the decision in the decision note. Step 2 waits for this evidence and the other locked prerequisites.
- Run the same command on the available backup with `--cohort direct`. Record the same smoke evidence in the PR for information only.
- Use the automated stopped worker and live worker start tests above as guard evidence. If a stopped worker has a retryable compaction card, click Retry compaction. Check that Bridge shows the refusal and the worker stays stopped. Record whether the optional app check took place.

## Out of Scope

- The prompt rule for read-only workers, its eval and its policy tests (step 2).
- The Codex `features.multi_agent` override and the OpenCode `task` deny (prerequisite d).
- A change to the result notice of a worker (prerequisite a) or to the evidence load (prerequisite b).
- A backfill of `token_estimate` and a weight for per-turn reads, cache reuse or provider replay.
- Codex `commandActions` as a source of a class.
