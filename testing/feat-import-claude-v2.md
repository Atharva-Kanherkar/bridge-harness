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
