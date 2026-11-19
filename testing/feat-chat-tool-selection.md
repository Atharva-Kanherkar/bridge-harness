# feat/chat-tool-selection: Test Contract

A chat starts with only the connectors and skills that the person keeps for it. Bridge stores the choice of the chat and translates it into "off" values for each harness. Locked before implementation. Plan note: the comment "Plan note: start each chat with only the connectors and skills it needs" on the issue of this change. Stack: on `feat/tool-result-ceiling`.

## Functional Behavior

- F1. A chat without a stored choice launches with the same sidecar config, the same Codex arguments and the same OpenCode environment as before.
- F2. Bridge stores only the choice of the chat, under the kind `chat_tool_selection_v1`, with its own revision. Bridge stores no inventory.
- F3. Connectors use an off list, so a new connector is on. Claude skills use an allowlist. A Claude skill that appears later shows as left out. An empty allowlist is distinct from `all`.
- F4. Identities are exact. Claude uses the display name of a server and the qualified name of a plugin skill (`plugin:skill`, as the init gives it). Codex uses the config key. OpenCode uses the `mcp` key.
- F5. The units are these. Claude: each MCP server and each skill. Codex: each MCP server of the effective config, `codex_apps` as one switch, and skills only when `config/read` shows no `skills` key. OpenCode: each MCP server.
- F6. Each item without a verified lever shows as fixed, with the reason. Fixed items: each app inside `codex_apps`, Codex plugins, Claude plugins as units, OpenCode skills, and Codex skills under a `skills` key.
- F7. The Bridge server stays as attached today. A choice that leaves out `bridge` is refused.
- F8. A choice only removes. No choice adds an allow rule beyond the set of today, removes a deny, or changes the permission mode, hooks, sandbox, write scope or approvals.
- F9. Claude: a skill list gives `Skill(<name>)` allow rules, a subset of `Skill`. An empty list gives no `Skill` rule. Left-out servers merge into the flag-tier `deniedMcpServers` as a union.
- F10. Codex: Bridge adds only `-c mcp_servers.<key>.enabled=false`, `-c features.apps=false` and `-c skills.config=[...]` with `enabled=false` entries.
- F11. OpenCode: Bridge sets only `mcp.<key>.enabled=false` leaves in `OPENCODE_CONFIG_CONTENT`. If the inherited value does not parse, Bridge does not apply the choice and the picker says so.
- F12. A new chat saves its first choice in the transaction of `sessions/create_configured_chat`, through the optional field `toolSelection`. A choice for another harness or account fails the create, and nothing commits.
- F13. `sessions/update_chat_tools` and a send share one guard. A save is refused while the chat is busy: `working`, `checkpointing`, an active turn, an unacknowledged send, a native subagent that waits, queued input or a queued worker result.
- F14. A save does not stop the process. The next turn launches again when the applied revision differs from the stored revision.
- F15. The relaunch resumes natively with the same provider session id.
- F16. If the native resume fails with a changed set, Bridge shows the error and keeps the provider session id. Bridge does not fork and does not start fresh.
- F17. Bridge records `session.tool_selection_changed` only after a launch with a different effective set succeeds. For Claude, the proof is the first turn on the same session id.
- F18. Forks and asides start with no choice. Workers, briefings and workflows ignore the choice. A choice for another harness or account is kept, labeled, and not applied.
- F19. The system prompt bytes stay the same after a change.
- F20. Items come from the connector catalog, the last Claude init of the chat, and the warm runtime of the chat. Bridge starts no process to list items.
- F21. Each token figure names its source. An item without a figure shows none.
- F22. The protocol version is 1.46. A 1.45 client still pairs with a 1.46 daemon.

## Unit Tests

Rust, `bridge-core`, stored choice and checks (`tool_selection.rs`):

- `tool_selection::tests::a_chat_without_a_row_launches_with_no_tool_launch`
- `tool_selection::tests::the_off_list_keeps_a_new_connector_on`
- `tool_selection::tests::the_claude_skill_allowlist_keeps_an_empty_set_distinct_from_all`
- `tool_selection::tests::a_new_claude_skill_shows_as_left_out`
- `tool_selection::tests::an_omission_of_bridge_is_refused`
- `tool_selection::tests::ids_with_control_characters_duplicates_or_too_many_bytes_are_refused`
- `tool_selection::tests::claude_skill_names_that_the_sdk_rejects_are_refused`
- `tool_selection::tests::a_codex_key_outside_the_safe_set_is_refused`
- `tool_selection::tests::the_serialized_row_stays_within_half_the_reader_cap`
- `tool_selection::tests::a_stale_revision_is_refused`
- `tool_selection::tests::a_choice_for_another_harness_or_account_is_kept_and_not_applied`
- `tool_selection::tests::a_saved_name_that_the_source_does_not_list_stays_visible`
- `agent_config::tests::reset_all_keeps_the_tool_selection`

Rust, units and fixed items:

- `tool_selection::tests::claude_units_are_each_server_and_each_skill`
- `tool_selection::tests::codex_units_are_each_configured_server_codex_apps_and_skills_without_a_table`
- `tool_selection::tests::opencode_units_are_each_server_and_skills_are_fixed`
- `tool_selection::tests::an_item_without_a_verified_lever_is_fixed_with_a_reason`
- `tool_selection::tests::the_bridge_server_is_fixed_on_for_each_harness`
- `apps::codex::tests::the_status_reply_keeps_a_token_estimate_for_each_server`
- `apps::codex::tests::skills_and_config_requests_are_reads_on_the_chat_thread`

Rust, launch arguments of each harness (each reads the real `Command` or config JSON):

- `claude_adapter::tests::a_chat_without_a_selection_writes_the_same_sidecar_config`
- `claude_adapter::tests::the_sidecar_config_carries_the_tool_selection_only_for_a_top_level_chat`
- `codex_adapter::tests::a_chat_without_a_selection_launches_with_the_same_arguments`
- `codex_adapter::tests::the_launch_arguments_turn_off_only_the_selected_servers`
- `codex_adapter::tests::the_launch_arguments_never_turn_a_server_on_or_touch_approvals_or_sandbox`
- `codex_adapter::tests::codex_apps_off_is_one_features_switch`
- `codex_adapter::tests::the_skills_override_is_absent_when_the_config_has_a_skills_key`
- `codex_adapter::tests::the_bridge_server_override_stays_after_the_selection`
- `opencode_adapter::tests::a_chat_without_a_selection_keeps_the_inherited_config_content`
- `opencode_adapter::tests::the_selection_sets_only_enabled_false_leaves_and_keeps_the_rest`
- `opencode_adapter::tests::an_inherited_config_that_does_not_parse_leaves_the_selection_unapplied`
- `opencode_adapter::tests::the_bridge_entry_and_each_permission_rule_stay`
- `adapters::tests::the_default_start_chat_and_resume_chat_ignore_the_tool_launch`

Rust, policy:

- `tool_selection::tests::no_selection_widens_a_permission_on_any_harness`. The test runs the three translations over generated choices. It asserts that the allow set is a subset of the baseline, and that the deny set contains the baseline. It also asserts that mode, hooks, sandbox, write scope and approvals are equal.

Rust, protocol:

- `handshake::tests::a_1_45_client_pairs_with_a_1_46_daemon`
- `chat_defaults::tests::create_configured_chat_params_accept_an_optional_tool_selection`
- The drift tests that exist: `checked_in_artifacts_match_the_contract` and `every_command_signature_matches_its_contracted_params`.

Sidecar, `sidecar/claude-agent/test/tool-selection.mjs` (each case reads the real spawn `args` and the first stdin control request through a `spawnClaudeCodeProcess` stub):

- `no selection leaves skills all and the flag settings unchanged`
- `a skill list gives Skill(name) allow rules that are a subset of Skill`
- `an empty skill list gives no Skill allow rule`
- `the init request carries the skill list only for a list`
- `omitted servers merge into deniedMcpServers and keep each existing deny`
- `the bridge server is never denied`
- `permission mode, hooks and the bridge allow list do not change in any write mode`
- `briefing and workflow options ignore the selection`
- `the context usage frame keeps the token figure of each skill`

Frontend, `src/components/ChatToolsControl.test.tsx` (jsdom):

- `shows a skeleton while the tools load`
- `shows the count of left-out items on the chip`
- `turns a left-out connector on and saves with the revision`
- `shows the busy refusal and keeps the old choice`
- `shows fixed items with the reason and no switch`
- `shows a choice for another harness as not applied`
- `shows a token figure with its source and none when absent`
- `hides the control for a harness without units`

Frontend, `src/App.test.tsx` (jsdom):

- `sends the draft tool choice inside createConfiguredChat and sends the first message after it`

## Integration and Functional Tests

First choice inside the create transaction:

- `chat_defaults::tests::create_saves_the_first_tool_selection_in_the_same_transaction`
- `chat_defaults::tests::create_refuses_a_tool_selection_for_another_harness_and_commits_nothing`
- `chat_defaults::tests::create_without_a_tool_selection_writes_no_row`

Shared busy guard:

- `api::tests::a_send_and_a_save_cannot_both_win`. A thread holds the send lease. The save is refused. Then a save holds the write lease, and a send waits and launches with the new revision.
- `tool_selection::tests::a_save_is_refused_while_the_chat_is_working_or_checkpointing`
- `tool_selection::tests::a_save_is_refused_with_queued_input_a_waiting_subagent_or_a_pending_worker_result`
- `tool_selection::tests::a_save_is_refused_for_a_worker_or_a_child_session`
- `tool_selection::tests::a_save_is_accepted_after_a_failed_launch`
- `tool_selection::tests::a_save_does_not_stop_the_process`
- `runtime_budget::tests::the_shared_idle_check_releases_the_same_sessions_as_before`

Relaunch keyed to the revision:

- `live_turn::tests::a_warm_chat_with_the_same_revision_is_reused`
- `live_turn::tests::a_changed_revision_relaunches_a_warm_chat_at_the_next_send`
- `live_turn::tests::a_changed_revision_relaunches_through_start_chat`
- `live_turn::tests::workers_briefings_and_workflows_launch_without_a_tool_launch`
- `live_turn::tests::a_fork_and_an_aside_launch_without_a_tool_launch`

Native resume on success and on failure:

- `live_turn::tests::a_changed_selection_resumes_natively_with_the_same_provider_session_id`
- `live_turn::tests::a_failed_resume_with_a_changed_selection_keeps_the_provider_session_id_and_does_not_fork_or_start_fresh`
- `live_turn::tests::a_failed_resume_without_a_selection_change_keeps_the_existing_fallback`
- `live_turn::tests::a_removed_codex_key_fails_the_launch_with_the_key_in_the_message`
- `live_turn::tests::a_restarted_launch_compiles_the_same_system_bytes`. This test exists (live_turn.rs:450). It must stay green.
- `live_turn::tests::a_changed_selection_delivers_the_same_system_bytes`

Event after a successful changed launch:

- `live_turn::tests::the_prefix_event_follows_a_successful_changed_codex_or_opencode_launch`
- `live_turn::tests::the_prefix_event_follows_the_first_claude_turn_on_the_same_session`
- `live_turn::tests::no_prefix_event_after_a_failed_launch_or_an_unchanged_set`
- `live_turn::tests::the_first_launch_with_a_first_choice_records_the_event_without_a_prefix_rewrite`

Daemon:

- `bridged` dispatch case `chat_tools_methods_answer_over_the_socket`. It calls both methods and the create field through the socket.

## Smoke Tests

- `bun run build` and `bun run test` are green.
- `cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets` and `node scripts/clippy-ratchet.mjs` pass.
- `cargo run --manifest-path src-tauri/Cargo.toml -p bridge-protocol --bin generate-protocol-artifacts` makes no diff after the commit.

## E2E Tests

Live checks. Run each one before the PR, and paste the output without edits. Set up the app first:

```bash
export BRIDGE_DATA_DIR="$HOME/bridge-t3-live"; DB="$BRIDGE_DATA_DIR/bridge.db"
BRIDGE_DATA_DIR="$BRIDGE_DATA_DIR" bun run tauri dev
B="src-tauri/target/debug/bridge exec --json --data-dir $BRIDGE_DATA_DIR"
ID=$(sqlite3 "$DB" "SELECT id FROM sessions WHERE parent_session_id IS NULL ORDER BY rowid DESC LIMIT 1")
sqlite3 "$DB" "SELECT provider_session_id FROM sessions WHERE id='$ID'"
sqlite3 "$DB" "SELECT sequence, payload FROM session_entries WHERE session_id='$ID' AND kind='session.tool_selection_changed'"
```

Claude (one server S and one skill K that the user has):

1. Make a new chat. In the picker, leave out S and keep only K. Send the prompt `Reply with the single word ok.`
2. Run `ps -ww -axo pid,args | grep -- --allowedTools | grep -v grep`. Expected: the list has `Skill(K)` and no bare `Skill`. The settings value has `deniedMcpServers` with S.
3. Run `sqlite3 "$DB" "SELECT json_extract(payload,'$.data.mcp_servers'), json_extract(payload,'$.data.skills') FROM session_entries WHERE session_id='$ID' AND kind='session.started' AND json_extract(payload,'$.data.subtype')='init' ORDER BY sequence DESC LIMIT 1"`. Expected: S is absent or not `connected`. The `skills` value still holds each skill, because the init does not apply the filter. The skill figure of the first `context_readings` row is lower than in a chat with no choice.
4. Turn S on. Send a turn. Expected: S is `connected` in the init, the provider session id is the same, and one new event has `prefixRewritten` true.
5. Move the transcript away with `mv ~/.claude/projects/<cwd key>/<provider id>.jsonl /tmp/t3-hold.jsonl`. Change the choice and send. Expected: an error in the chat, the same provider session id, and no new event. Move the file back and send. Expected: the chat resumes.
6. Add `{"deniedMcpServers":[{"serverName":"<S2>"}]}` to `~/.claude/settings.json`. Leave out S in the chat. Expected: S2 stays denied.

Codex (one server X in `~/.codex/config.toml`, no `skills` key):

1. Send one turn. Run `$B --method sessions/get_chat_tools --params '{"sessionId":"'$ID'"}'`. Expected: X, `codex_apps` and the skills are listed, each figure with the source `codex.mcpServerStatus` or `codex.skills/list`.
2. Turn X and one skill off. Send a turn. Run `ps -ww -axo pid,args | grep 'app-server' | grep -v grep`. Expected: `-c mcp_servers.X.enabled=false` and `-c skills.config=[{name="<skill>",enabled=false}]`. The provider session id is the same.
3. Run the `get_chat_tools` call again. Expected: X reports no tools, and the skill reports `enabled` false.
4. Turn `codex_apps` off and send. Expected: `-c features.apps=false` in the arguments, and `codex_apps` is absent from the status list.
5. Remove X from `config.toml` and send. Expected: an error that names X, the same provider session id, no new session row, and no fork.
6. Add a `[[skills.config]]` entry to `config.toml` and open the picker. Expected: each skill shows as fixed, with the reason.
7. Do steps 1 to 3 with the pinned runtime and with the installed `codex`. Record `codex --version` for each.

OpenCode (one server Y defined in `~/.config/opencode/opencode.json` with `node testing/fixtures/fake-mcp-server.mjs`):

1. Send one turn. Run `$B --method sessions/get_chat_tools --params '{"sessionId":"'$ID'"}'`. Expected: Y is listed with no token figure.
2. Turn Y off and send. Run `ps eww -axo pid,args | grep OPENCODE_CONFIG_CONTENT | grep -v grep`. Expected: `"Y":{"enabled":false}` and the Bridge entry stays. Run `pgrep -f fake-mcp-server`. Expected: no process. The provider session id is the same.
3. Turn Y on and send. Expected: `pgrep -f fake-mcp-server` shows one process.
4. Start the app with `OPENCODE_CONFIG_CONTENT='{bad'` and open the picker. Expected: the choice shows as not applied, with the reason.
5. Do steps 1 to 3 on the pinned 1.18.32 (V1) and on a 2.x binary if one is available.

## Manual Tests

- Take three screenshots of `ChatToolsControl` for the PR: `chat-tools-dark-closed`, `chat-tools-light-open` and `chat-tools-light-toggle-pending`.
- Try a save while a turn runs. Expected: the list shows the refusal, and the choice stays the same.
- Run the two before and after queries of the plan note for the three harnesses. Paste the output without edits.

## Out of Scope

- Automatic selection by a model.
- Discovery, sign-in and configuration of connectors (a separate change).
- A process that lists items for a draft. The named reads stay open.
- Each app inside `codex_apps`, Codex plugins and OpenCode skills. No verified lever exists.
- A change to the capability text that names the skill roots.
- A smaller set on a native resume without a relaunch of the harness.
