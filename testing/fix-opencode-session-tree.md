# fix/opencode-session-tree — Test Contract

Closes the three OpenCode defects that share one root cause (issue "Fix OpenCode:
subagent sessions, unattributed errors, and no chat-turn watchdog"): the SSE
reader in `opencode_adapter.rs` admits a frame only when `properties.sessionID`
equals the root session id, so subagent child sessions, unattributed
`session.error` frames, and `server.heartbeat` are all dropped without a trace.

Verified against the installed OpenCode 1.18.31 binary (`strings` over
`opencode-darwin-arm64/bin/opencode`): `server.heartbeat` is
`{id, type:"server.heartbeat", properties:{}}` on a 10 s tick with no session
id; `GET /session/status` and `GET /session/{id}/message` exist; the
`session.error` schema keeps `sessionID` optional.

## Functional Behavior

### D1 — session-tree attribution
- The reader owns a *set* of session ids seeded with the root id. A
  `session.created` or `session.updated` frame whose `properties.info.parentID`
  is in the set adds `properties.info.id` to the set and is forwarded.
- Any frame whose `properties.sessionID` is in the set is forwarded. The flat
  `properties.sessionID` is the primary location (runtime schema); the nested
  `properties.part.sessionID` / `properties.info.sessionID` are accepted as a
  fallback only.
- Frames from a child session normalize with `data.subagent =
  {sessionId, agent?, title?}` on every emitted event, so the transcript can
  label them instead of interleaving them as the parent's own work.
- Child-session lifecycle frames never drive the parent turn: a child's
  `session.status`, `session.idle`, `session.created`, `session.updated`,
  `todo.updated`, and `session.compacted` emit nothing. A child's
  `session.error` emits an `error` event with status `warning` and no
  `turn.completed` sibling.
- A child's `permission.asked` / `question.asked` are forwarded and normalize
  exactly like the parent's (the reply endpoints are id-keyed).
- Frames matching none of the rules are dropped, counted in the frame queue
  metrics (`dropped_foreign`), and logged once per distinct session id.

### D2 — unattributed `session.error`
- A `session.error` frame with no `properties.sessionID` is forwarded and
  normalizes to the existing `error` + failed `turn.completed` pair.

### D3 — heartbeat and chat-turn watchdog
- `server.heartbeat` and `server.connected` are forwarded. The normalizer emits
  nothing for them (no `provider.unknown` row) and the reader thread does not
  count them as progress.
- Every session's last progress frame is tracked in memory (the existing
  `worker_activity` map, now fed for all sessions). SQLite persistence stays
  worker-only.
- A depth-0 session in status `working` whose provider has emitted no progress
  frame for `CHAT_STALL_TIMEOUT_SECONDS` (600) — or
  `CHAT_TOOL_STALL_TIMEOUT_SECONDS` (1800) while a tool/command/file-change
  item of the current turn is still open — is resolved: the adapter is
  interrupted (best effort), an `error` event (title "Turn stalled", status
  `failed`, `data.bridgeStall = true`) and a failed `turn.completed` are
  persisted and published, the session returns to `ready` with no active
  turn, and the adapter stays alive so the next message just works.
- Sessions in `waiting` (approval pending), `checkpointing`, or any non-working
  status are never stalled. Workers (depth > 0) keep the existing worker
  watchdog untouched.
- The error frame the interrupt provokes from the provider is swallowed via the
  existing `user_stop_requested` path, so the stall renders exactly one card.

### D4 — terminal signal
- A turn completes from `session.status {type: idle}` alone. `session.idle`
  remains accepted as a fallback and never emits a second `turn.completed`.

### D5 — SSE reconnect
- When the `/event` body ends while the runtime is not shutting down, the
  reader reconnects with bounded backoff (1, 2, 4, 8, 16 s; 5 attempts). While
  the retries run no `session.error` is synthesized.
- After a successful reconnect the reader resyncs: it re-emits the latest
  assistant message's `message.updated` and `message.part.updated` snapshots
  from `GET /session/{id}/message`, then reads `GET /session/status` and emits
  a synthetic `session.status {idle}` when the root session is no longer
  busy — so a turn that finished inside the gap still closes.
- When every retry fails, the existing "event stream disconnected
  unexpectedly" `session.error` is synthesized exactly once.

### D6 — evidence over SDK types
- `testing/fixtures/opencode-sse-session-tree-v1.json` holds an SSE transcript
  (parent turn, `task` child session, child frames, unattributed error,
  heartbeat, foreign-session frame) with the expected disposition of every
  line. Its provenance names which lines came from a live 1.18.31 capture and
  which were assembled from the schema shapes.

## Unit Tests

Rust (`cargo test -p bridge-core`):
- `opencode_adapter::tests::filter_admits_the_root_and_its_descendants_and_counts_the_rest` — set seeding, transitive child admission, foreign counted.
- `opencode_adapter::tests::filter_admits_unattributed_session_errors_and_heartbeats` — D2/D3 admission.
- `opencode_adapter::tests::filter_reads_the_flat_session_id_first` — D6 pin: the runtime shape is primary; the SDK's nested shape is a fallback.
- `opencode_adapter::tests::session_tree_fixture_replays_with_the_recorded_dispositions` — every fixture line classifies as recorded.
- `opencode_adapter::tests::stream_reconnects_after_a_dropped_body_and_resyncs_the_turn` — fake loopback server: first `/event` body ends after one frame, second serves the rest; `/session/{id}/message` and `/session/status` answer the resync; the consumer sees the replayed snapshot and a synthetic idle. Tiny injected backoff.
- `opencode_adapter::tests::stream_gives_up_after_the_retry_budget_with_one_disconnect_error` — a server that refuses reconnects yields exactly one synthetic `session.error`.
- `agent::tests::opencode_child_session_frames_are_tagged_as_subagent_work` — child text/tool events carry `data.subagent`; child lifecycle emits nothing; child error is a warning.
- `agent::tests::opencode_unattributed_session_error_still_fails_the_turn` — D2 normalization.
- `agent::tests::opencode_heartbeat_normalizes_to_nothing` — D3.
- `agent::tests::opencode_turn_completes_from_session_status_idle_alone` — D4.
- `adapters::tests::opencode_registry_routes_child_frames_to_the_root_stream_state` — registry keys child frames by root id; `forget_session` purges the mapping.
- `frame_queue::tests::foreign_drops_are_counted` — metrics field.
- `live_turn::tests::a_silent_chat_turn_is_resolved_to_a_recoverable_error` — depth-0 working session silent past 600 s → interrupt sent, error + failed turn.completed persisted, status `ready`, adapter still registered.
- `live_turn::tests::a_running_tool_extends_the_chat_stall_deadline` — silent 700 s with an open `tool.started` → untouched; 1900 s → stalled.
- `live_turn::tests::waiting_and_worker_sessions_are_not_chat_stalled` — `waiting` status and depth-1 sessions are left to their own paths.
- `live_turn::tests::heartbeat_frames_do_not_refresh_progress` — a `server.heartbeat` line leaves the activity timestamp alone and reaches no handler.

Frontend (`bunx vitest run`):
- `src/transcript/reducer.test.ts` — `data.subagent` survives on tool, message and reasoning items.
- `src/components/AgentConversation.test.tsx` — a tool row from a child session renders the subagent label; a parent row renders none.

## Integration / Functional Tests

- `adapters::tests` registry test above exercises adapter → normalizer routing end to end with real frames.
- The reconnect tests run the real `spawn_event_stream` thread against a loopback HTTP server, so the SSE parser, queue, backoff and resync run unmodified.

## Smoke Tests

- `bun run check` (tsc + cargo check) green.
- `bun run test` green.
- `cargo run -p bridge-protocol --bin generate-protocol-artifacts` produces no diff (no wire types change).

## E2E Tests

N/A — a live subagent run needs a paid model. Manual path below.

## Manual / cURL Tests

1. Start a private server and watch the bus:
   ```bash
   opencode serve --port 4096 --hostname 127.0.0.1 &
   curl -N http://127.0.0.1:4096/event
   ```
   Expect `server.connected`, then `server.heartbeat` every ~10 s with
   `properties: {}` and no `sessionID`.
2. Create a parent and a child:
   ```bash
   P=$(curl -s -X POST http://127.0.0.1:4096/session -H 'content-type: application/json' -d '{"title":"parent"}' | jq -r .id)
   curl -s -X POST http://127.0.0.1:4096/session -H 'content-type: application/json' -d "{\"title\":\"child\",\"parentID\":\"$P\"}"
   ```
   Expect a `session.created` frame whose `properties.info.parentID` is `$P`.
3. In Bridge, run an OpenCode chat that uses the `task` tool: the child's
   text and tool cards appear labelled as subagent work, and the parent turn
   completes exactly once.
4. Kill the OpenCode server mid-turn (`kill -9`): exactly one "event stream
   disconnected unexpectedly" card after the retries, never a spinner.

---

# Addendum — GitHub PR reviewer: OpenCode launch and reviewer settings

Added to the same PR after the first review. The GitHub pane's "Review with
OpenCode" failed with "opencode cannot run a read_only worker", and the
reviewer's model, effort and instructions were not user-configurable.

## Functional Behavior

- `github/github_review` with `opencode` launches the review worker with
  `writeMode: isolated` instead of `readOnly` (OpenCode's HTTP transport
  cannot run in the offline sandbox, by design). A research worker with no
  owned paths goes through the policy engine's approval gate, so the result
  is `awaitingApproval` with a message that says so; it never returns
  `failed` for this reason. Claude and Codex keep `readOnly`.
- New global reviewer settings (`config/get_reviewer_settings`,
  `config/save_reviewer_settings`): per harness (`claude`, `codex`,
  `opencode`) an optional model id and optional effort, plus an optional
  system prompt. Stored in `configuration_entries` under
  `kind='reviewer_settings'`, `id='global'`.
- `github_review` resolves, in order: the reviewer setting for the chosen
  harness, then the Reviewer model profile (model only when its provider is
  the chosen harness), then the harness tier default. Effort resolves the same
  way. An empty system prompt means Bridge's default review instructions; a
  non-empty one replaces them, with `{number}` expanded to the PR number.
- Saving rejects unknown harness ids and a prompt longer than 20 000 chars.
- Settings → Workers gains a "Pull request reviewer" group: model and effort
  selects per harness (models from each adapter's catalog, "Harness default"
  and "Profile default" as the empty choices), a prompt textarea whose
  placeholder is the default text, and a reset control.
- The GitHub pane's harness menu notes that OpenCode runs in an isolated
  worktree and needs an approval.

## Unit Tests

- `reviewer_settings::tests::round_trips_and_defaults` — load with nothing stored is the default; save then load returns the saved value.
- `reviewer_settings::tests::rejects_unknown_harnesses_and_oversized_prompts`.
- `reviewer_settings::tests::objective_uses_the_default_or_the_custom_prompt_with_the_number_expanded`.
- `api::tests::reviewer_launch_plan_prefers_settings_then_profile_then_defaults` — pure planning function: model/effort precedence and OpenCode's isolated write mode.
- Frontend `src/components/settings/ReviewerSettingsSection.test.tsx` — loads settings, edits model/effort/prompt, saves, and never reports a failed save as saved.
- Frontend `src/components/GitHubPane.test.tsx` — the OpenCode menu item carries the isolated-worktree note.

## Smoke Tests

- `bun run check`, `bun run build`, `bun run test` green; protocol artifacts regenerated (new schemas checked in).

## Manual

- Settings → Workers → Pull request reviewer: pick a model and effort for Codex, save, reload settings and see them persist.
- GitHub pane → Review → OpenCode: an approval appears on the conversation instead of a launch failure; approving it starts the worker.
