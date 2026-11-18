# Test contract: import transfer bundle, resolver and writer

Branch `feat/import-transfer-bundle`. Issue #1051, epic #1050.

Every import source converts into one `TransferBundle`. The resolver merges
bundles so one native session is one Bridge chat. The writer stores the whole
chat. These expectations were locked before the writer changed and are each
pinned by a named test in `bridge-core/src/import_bundle/tests.rs` unless noted.

## Resolver

| # | Expectation | Test |
|---|---|---|
| R1 | Chats from different tools with the same `harness:sessionId` merge into one; every contributing source is kept | `one_native_session_from_three_tools_is_one_chat_with_three_sources` |
| R2 | Entries come from one source only: native over a tool copy, never interleaved | same |
| R3 | Title precedence: typed by the person > tool > native > derived | same |
| R4 | A tool copy fills in when the native transcript is gone | `a_tool_copy_fills_in_when_the_native_transcript_is_gone` |
| R5 | A bundle version this build does not know fails closed with the version in the message | `a_newer_bundle_version_fails_closed` |
| R6 | Parents are ordered before children; a missing or cyclic parent leaves the chat top-level with a diagnostic | `subagent_chats_import_as_children_of_their_parent`, `a_missing_or_cyclic_parent_keeps_the_chat_at_the_top_level` |
| R7 | Workspaces and projects merge by real path, `..` and trailing slashes resolved | `workspaces_and_projects_merge_by_real_path` |
| R8 | Duplicate setup items collapse to one | `setup_and_preferences_import_inert` |

## Writer

| # | Expectation | Test |
|---|---|---|
| W1 | The chat keeps its harness, cwd, model, archive state and a source label; it is never hardcoded to Claude | `every_entry_kind_lands_in_the_forest_and_renders_as_its_live_twin` |
| W2 | User, assistant, reasoning, tool call with result (completed or failed), image attachment and compaction each become a validated forest entry, in order | same |
| W3 | A chat whose folder matches a workspace path is filed under it and its project hint is confirmed | `a_chat_is_filed_under_the_workspace_whose_path_matches_its_folder` |
| W4 | A subagent chat is a child session of its parent with depth 1 | `subagent_chats_import_as_children_of_their_parent` |
| W5 | Re-import of the same file is a no-op; the same session through another tool is a no-op; a changed session needs an explicit choice | `reimporting_through_another_tool_is_a_no_op_and_a_changed_chat_asks_first` |
| W6 | A session Bridge itself ran is never imported | `a_session_bridge_ran_itself_is_never_imported` |
| W7 | Secrets in text, tool input and tool output never reach the database | `secrets_in_messages_and_tool_io_never_reach_the_database` |
| W8 | Oversized images, empty messages and empty chats are counted in `excluded`, not dropped silently | `oversized_images_and_empty_messages_are_reported_not_dropped_silently` |
| W9 | Tool output over 20,000 characters is cut with a visible marker | `long_tool_output_is_cut_with_a_visible_marker` |
| W10 | An unknown harness is refused and nothing is half-written | `an_unknown_harness_is_refused_instead_of_stored` |
| W11 | Memories from another tool carry that tool in their provenance | `memories_from_another_tool_carry_that_tool_in_their_provenance` |
| W12 | A failure after any single write rolls back every table | `a_failure_after_any_write_leaves_nothing_behind` |
| W13 | Existing Claude import behaviour is unchanged | existing `external_import::tests` and `claude_import::tests`, unmodified |

## Fuzz

`arbitrary_bundles_resolve_lower_and_commit_atomically` (proptest, 256 cases):
any mix of bundles from three tools, with colliding native ids and keys,
random parents, empty and unicode text, resolves and lowers without
panicking; identities are unique; parents precede children; sequences are
contiguous from 1; a commit either writes exactly `imported` sessions with no
orphaned parents or writes nothing.

## Mutation

`cargo mutants` on `import_bundle.rs` with the tests above. Every surviving
mutant is either killed by a new test or listed in the PR with the reason it
is equivalent.

## UI

The imported chat's toolbar badge names the source tool from the entry's
`importSource` (`src/importSources.test.ts`).

## Review amendments

Added after review of #1064. Each row names the regression test that failed before its fix.

| # | Expectation | Test |
|---|---|---|
| A1 | A secret in tool output is redacted before the output is cut, so a private key straddling the 20,000-character cap never survives | `a_secret_cut_by_the_output_cap_is_redacted_before_cutting` |
| A2 | Archiving a workspace succeeds when imported chats are filed under it: they stay as history with their folder, and project hints stop pointing at the workspace | `archiving_a_workspace_keeps_its_imported_chats_as_history` |
| A3 | A child whose parent was skipped (already imported, or changed) attaches to the parent's existing Bridge chat | `a_child_of_a_skipped_parent_attaches_to_the_existing_parent` |
| A4 | A native session Bridge ran stays Bridge's after the chat switches harness, so importing it later is a no-op | `a_session_bridge_ran_stays_bridges_after_a_harness_switch` |
| A5 | Setup items deduplicate per scope and project, and the inert candidate keeps its project | `setup_is_deduplicated_per_project_and_keeps_its_project` |
| A6 | The image cap applies to decoded bytes: a 4 MiB image imports, one byte over 5 MiB is excluded | `the_image_cap_counts_decoded_bytes` |
| A7 | A historical revision imported through another tool records the chat it revises | `a_cross_tool_revision_records_its_predecessor` |
| A8 | An imported compaction is a native `context.compacted` entry (drawn as "Context compacted"), never a Bridge checkpoint | `an_imported_compaction_is_native_not_a_bridge_checkpoint` |

### Second review

| # | Expectation | Test |
|---|---|---|
| A9 | A child whose parent is a session Bridge ran (skipped as Bridge's own) attaches to that Bridge chat | `a_child_of_a_bridge_run_parent_attaches_to_that_chat` |
| A10 | Setup conflicts compare kind, name, scope and project: the same script in two projects is not a conflict, the same script in one project still is | `setup_conflicts_are_per_scope_and_project` |
| A11 | Memories deduplicate by their real destination project, not a tool-local project key | `memories_deduplicate_by_the_real_project` |
