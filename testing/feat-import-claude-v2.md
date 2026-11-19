# Test contract: Claude Code import v2

Branch `feat/import-claude-v2`, stacked on `feat/import-transfer-bundle`. Issue #1053, epic #1050.

Claude Code transcripts import at the fidelity the person saw them, through
the transfer bundle. Shapes come from a survey of a real store (81,567
transcripts, 1.57 M lines) and a real Claude Code 2.1.325 session recorded for
this branch.

| # | Expectation | Test (`claude_import/transcript/tests.rs` unless noted) |
|---|---|---|
| C1 | User text, thinking, tool calls with results (failed and completed), image results, assistant text and user images map to their entries in order | `every_block_kind_maps_to_its_entry` |
| C2 | A `/rename` title wins over the AI title; an AI title not grounded in what the person said is not used | same, `an_ungrounded_ai_title_is_not_used` |
| C3 | A compaction is one boundary entry carrying the summary, with or without a boundary record before it | `compaction_becomes_one_boundary_carrying_the_summary` |
| C4 | Meta records and bookkeeping are skipped silently; unknown record types, unknown system subtypes, malformed lines and non-objects are counted | `meta_records_bookkeeping_and_slash_commands_are_handled` |
| C5 | Slash commands read as typed (`/model opus`) | same |
| C6 | Bad timestamps, missing ids, repeated ids and results without a call are counted, never fatal; a call with no result is `interrupted` | `bad_timestamps_missing_ids_and_orphan_results_are_counted_not_fatal` |
| C7 | Each `subagents/agent-*.jsonl` is a child chat of its session, titled from its type and task | `subagent_transcripts_become_child_chats` |
| C8 | The review list peeks title, folder and start time from the first and last 64 KiB only | `peek_reads_title_folder_and_start_without_the_whole_file` |
| C9 | A single line over 64 MiB is skipped without loading it | `a_huge_single_line_is_skipped_without_reading_it_into_one_string` |
| C10 | Discovery lists a session once; its subagent files are not sessions of their own | `claude_import::tests::a_real_session_imports_with_its_subagent_tools_thinking_and_image` |
| C11 | A real session imports end to end: main chat plus Explore child, harness and folder kept, entry count equals what the reader saw, entry sequence matches the golden file, re-import writes nothing | same |
| C12 | Every Claude config folder (home, `CLAUDE_CONFIG_DIR`, accounts) is found once | `claude_import::tests::config_roots_cover_home_env_and_accounts_once_each` |
| C13 | Pre-v2 behaviour that still holds keeps its tests; the three that changed (tool calls and thinking now imported, ids now `uuid:block`) are updated in place with the reason | `claude_import::tests` |

## Fuzz

`arbitrary_lines_never_panic` (proptest, 200 cases): random bytes, random
unicode and mutated real-shaped records never panic the reader or the peek;
every entry has a valid timestamp; tool entries and tool payloads match; the
count of non-boundary entries equals the source items the reader counted.

## Real store

`real_store_sweep` (ignored; run with `BRIDGE_IMPORT_SWEEP`): reads every
session on a machine, lowers each through the bundle and commits every 40th to
a real store. Must report zero read errors, zero parity breaks and zero commit
errors. Prints aggregates only, never content.

## Review amendments

Added after review of #1065. Each row names the regression test that failed before its fix.

| # | Expectation | Test |
|---|---|---|
| B1 | A subagent transcript or its `.meta.json` that resolves outside the approved folders (a symlink) is never opened; the escape is reported | `claude_import::tests::a_subagent_symlink_outside_the_approved_folder_is_not_read` |
| B2 | A chat imported by the v1 reader is recognised by v2: the unchanged file is a duplicate, a changed one asks first, and the native mapping is filled in | `claude_import::tests::a_chat_imported_before_the_upgrade_is_not_imported_twice` |
| B3 | Commit re-derives exactly the artifacts the preview read, for every caller, so a copy that was not selected cannot change the candidates | `api::tests::commit_uses_the_artifacts_the_preview_read` |
| B4 | Inline sidechain records (`isSidechain` with `agentId` in the session file) become child chats, and a child also present as a `subagents/` file appears once | `inline_sidechains_become_child_chats`, `an_inline_sidechain_and_its_file_are_one_child` |
| B5 | A subagent transcript that cannot be read is reported against its session instead of disappearing | `claude_import::tests::an_unreadable_subagent_is_reported` |
| B6 | Skipping an oversized line holds at most one buffer, not the rest of the line | `discarding_a_line_never_holds_more_than_one_buffer` |

### Second review

| # | Expectation | Test |
|---|---|---|
| B7 | A v1 import whose file later grew past 10 MiB is still recognised (as changed), not imported again | `claude_import::tests::a_v1_import_that_grew_past_ten_mib_is_still_recognised` |
| B8 | Every merged copy of a session is checked against the v1 ledger, not only the copy chosen as primary | `claude_import::tests::a_v1_import_of_any_copy_is_recognised` |
| B9 | A child of a changed v1 parent attaches to that parent's existing chat | `claude_import::tests::a_child_of_a_changed_v1_parent_attaches_to_it` |
| B10 | An inline subagent is dropped only for a readable file copy at least as long; otherwise the inline copy is kept | `an_inline_child_is_kept_when_its_file_is_unreadable_or_shorter` |
| B11 | Each chat keeps its own loss diagnostics when an inline child shares its file | `claude_import::tests::a_parent_keeps_its_losses_next_to_an_inline_child` |
| B12 | A transcript with no `sessionId` gets no shared native identity, so two unrelated files never merge | `files_without_a_session_id_never_merge` |
