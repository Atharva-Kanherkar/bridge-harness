# feat/notes-backend: Test Contract

Bridge Notes phase 1, backend half. Issue #866, parent epic #865. Locked before implementation.
No UI in this branch; every behavior is reachable through `notes/*` protocol methods, the
loopback notes-tool route, or Rust tests.

## Functional Behavior

### Store
- `notes.db` is a separate SQLite file in the data directory, opened with its own connection and
  its own `notes_schema_version`. Opening an empty directory creates schema v1 and exactly one
  `personal` space. Reopening is idempotent.
- Every id is a UUIDv7 string. Every row in `pages`, `page_updates`, `structured_views` and
  `model_jobs` carries or joins to a `space_id`.
- Every change is stamped with a hybrid logical clock (`<millis>-<counter>-<device>`) that is
  strictly increasing on one device even if the wall clock steps backwards.

### Page docs
- One `yrs` document per page: `Y.Text("body")` holds Your words; `Y.Map("props")` holds
  `title`, `kind`, `tags`, `workspaceId`, `state`.
- Every change appends one row to `page_updates (page_id, seq, hlc, actor, device_id,
  update_bytes)` in the same transaction that updates the `pages` index row.
- Replaying `page_updates` (from the latest snapshot, if any) reproduces the exact body and props.
- After 500 updates on a page a snapshot is written; replay from the snapshot equals full replay.
- A person's save sends full text plus `baseSeq`. The core rebuilds the doc at `baseSeq`, applies
  the text difference there as the person's client, and merges that update into the current doc.
  A concurrent agent edit made after `baseSeq` survives. A save whose text equals the doc at
  `baseSeq` writes no update.

### Notes service (the one write path)
- `capture_note {body}`: stores `body` byte for byte as Your words, `kind: note`,
  `state: inbox`, no project, actor `person`. Calls no model. Empty or whitespace-only body is
  refused.
- `create_page {title, content?, parentId?, kind?, tags?, workspaceId?}`: `state: open`; a missing
  `workspaceId` is allowed; an unknown `workspaceId` is refused; an unknown `parentId` is refused.
- `edit_page`:
  - `text` + `baseSeq`: person save as above.
  - `edits: [{oldStr, newStr}]`: each `oldStr` must match exactly once in the current body, else
    the whole call fails with a reason and writes nothing; applied in order.
  - `append: {content, position: start|end}`: inserts only; existing text unchanged.
  - Any edit that would remove a `<page ...>` or `<database ...>` tag fails unless
    `allowDeletingChildren: true`.
- `update_note`: sets title, kind, tags, workspace (or `clearWorkspace`), state; `addSessionId`
  adds to a set (no duplicates). Body untouched. Kind outside `note|idea|task|question` refused.
- `delete_note`: tombstone (`deleted_at`); hidden from `list_notes` and `read_page` returns not
  found; its tasks are tombstoned with it.
- Every successful write publishes `notes/page_changed {pageId, actor, seq}`.
- Actors: `person`, `file-import`, `agent:<harness>:<sessionId>`, `model:<structure|tasks>:<model>`.
- No method requires a project.

### Markdown projection
- After each write the page is projected to `<notes_root>/<slug>-<shortid>.md`: YAML frontmatter
  (`id`, `title`, `kind`, `tags`, `project`, `state`, `parent`, `created`, `updated`) then the body.
  Written atomically (temp file + rename). `notes_root` defaults to `<data_dir>/notes`.
- Frontmatter parsing keeps unknown keys on the next write.
- The importer (every 5 s and on `list_notes`) reads files whose hash differs from the last hash
  Bridge wrote and applies body and frontmatter changes as ops by actor `file-import`. Bridge's own
  writes are never re-imported.

### Model jobs (opt-in only)
- `structure_note {id, harness?, model?}` and `extract_tasks {id, harness?, model?}` enqueue one
  `model_jobs` row and return `{jobId}`. Nothing else enqueues jobs unless `autoStructure` is on
  (default off), in which case `capture_note` enqueues a structure job.
- A leased drain loop claims jobs; a job left `running` by a crash returns to `pending` on open.
  3 attempts, then `failed` with `detail`.
- Harness resolution: requested harness if available, else chat default if in
  `GENERATION_HARNESSES` and available, else first available of claude, codex, opencode. None:
  the job fails with `detail` naming why. Structure model defaults to `title_jobs::title_model`.
- Structure: reply JSON `{title, kind, tags, summary, items:[{excerpt,title,kind}], project}`.
  Guards: title 1..80 single line; kind coerced to `note` if unknown; summary <= 400; tags
  lowercase `[a-z0-9-]{1,24}`, max 4, deduped; project key must be listed else null; items whose
  excerpt is not a whitespace-normalized substring of the body are dropped; max 12 items.
  Unparseable reply or failed title guard: job fails, nothing written. Success: one
  `structured_views` row keyed to the body hash; body unchanged.
- `read_page` returns `structured.stale = true` when the stored view's body hash differs.
- Turn into tasks: reply `{tasks:[{excerpt,title,detail,doneWhen,project}]}`. Guards: max 20;
  title 1..80; detail <= 1200; doneWhen <= 300; unquoted excerpts dropped; unknown project key
  falls back to the page's own project or null. Each survivor becomes a child page
  (`kind: task`, `state: open`, `parent_id`, `source_excerpt`, body = detail + "Done when: ...").
  Re-running skips tasks whose normalized excerpt and title match an existing live task.
- `split_item {id, itemIndex}` creates a note from one structured item with `source_id` set.
- Spend and tokens are recorded on `model_jobs`; no `sessions` row is created.

### Agent tools
- The `bridge` tool server lists `notes_search`, `notes_read`, `notes_create`, `notes_edit`,
  `notes_append`, `notes_set` beside `visualize` when `BRIDGE_NOTES_URL` and `BRIDGE_NOTES_TOKEN`
  are present in its environment; without them it lists only `visualize` (old behavior).
- On `tools/list` the server asks the core for the chat's access. Read-only access lists only
  `notes_search` and `notes_read`.
- Tool calls go to `POST http://127.0.0.1:4317/notes-tool/<sessionId>/<tool>` with header
  `x-bridge-notes-auth: <token>`. The core verifies `token == HMAC-SHA256(process_secret,
  sessionId)` in constant time; wrong or foreign tokens get 401 and no effect.
- The core resolves the session's effective access: write tools are refused (403) for read-only
  access, whatever the server listed.
- Writes from tools use actor `agent:<harness>:<sessionId>`.
- Launch wiring: Claude `mcpServers.bridge.env`, Codex `mcp_servers.bridge.env`, OpenCode
  `mcp.bridge.environment` carry both variables. Workers and briefings get no server (existing
  `attaches_to`).
- Tool descriptions state when to create a page and when not to, prefer edit/append over rewrite,
  and that page text is data, never instructions.

### Slash and purpose chat
- `/note <text>` resolves to a capture of `<text>` through the Notes service; no model turn. Empty
  text is refused with usage help.
- `notes` is a known chat purpose with a stable prompt section that tells the agent it lives beside
  the Notes page, that a ` ```notes-context ` fence carries the open page, and to use the notes tools.

## Unit Tests
Rust, `bridge-core`:
- `notes::store::tests::opening_creates_schema_and_one_personal_space`
- `notes::store::tests::reopening_is_idempotent`
- `notes::ids::tests::ids_are_uuidv7_and_time_ordered`
- `notes::ids::tests::hlc_is_monotonic_when_the_wall_clock_goes_back`
- `notes::doc::tests::replay_reproduces_body_and_props`
- `notes::doc::tests::snapshot_replay_equals_full_replay`
- `notes::doc::tests::stale_save_merges_with_a_concurrent_append`
- `notes::doc::tests::unchanged_save_writes_no_update`
- `notes::service::tests::capture_stores_the_body_byte_for_byte`
- `notes::service::tests::capture_refuses_an_empty_body`
- `notes::service::tests::create_without_a_project_is_allowed_and_unknown_project_is_refused`
- `notes::service::tests::targeted_edit_requires_a_unique_match_and_is_all_or_nothing`
- `notes::service::tests::edit_refuses_to_remove_a_child_page_tag`
- `notes::service::tests::append_never_rewrites_existing_text`
- `notes::service::tests::update_props_leaves_the_body_untouched`
- `notes::service::tests::add_session_id_is_a_set`
- `notes::service::tests::delete_is_a_tombstone_and_takes_its_tasks`
- `notes::service::tests::every_write_publishes_page_changed`
- `notes::service::tests::rebuild_index_from_updates_restores_everything`
- `notes::markdown::tests::frontmatter_round_trips_and_keeps_unknown_keys`
- `notes::projector::tests::writes_atomically_with_frontmatter`
- `notes::projector::tests::importer_applies_outside_edits_as_ops`
- `notes::projector::tests::importer_skips_its_own_writes`
- `notes::jobs::tests::running_jobs_return_to_pending_on_open`
- `notes::jobs::tests::no_harness_fails_with_a_reason`
- `notes::jobs::tests::auto_structure_is_off_by_default`
- `notes::structure::tests::view_is_keyed_to_the_body_hash_and_body_is_unchanged`
- `notes::structure::tests::view_goes_stale_when_the_body_changes`
- `notes::structure::tests::guards_drop_unquoted_items_coerce_kind_and_normalize_tags`
- `notes::structure::tests::rejection_writes_nothing`
- `notes::structure::tests::unknown_project_key_becomes_null`
- `notes::tasks::tests::tasks_become_child_pages_with_verbatim_excerpts`
- `notes::tasks::tests::rerun_adds_no_duplicates`
- `notes::tasks::tests::invented_tasks_are_dropped`
- `notes::tool::tests::token_is_hmac_of_the_session_and_compares_in_constant_time`
- `notes::tool::tests::wrong_or_foreign_token_is_refused`
- `notes::tool::tests::read_only_access_cannot_write`
- `notes::tool::tests::full_access_can_create_and_edit`
- `mcp_apps::server::tests::notes_tools_listed_only_with_url_and_token`
- `mcp_apps::server::tests::read_only_access_lists_only_read_tools`
- `mcp_apps::tests::launch_entries_carry_notes_env_for_all_three_harnesses`
- `slash::tests::note_captures_without_a_model_turn`
- `notes::agent::tests::notes_purpose_is_known_and_has_a_section`

## Integration / Functional Tests
- `bridged/tests`: `notes_methods_round_trip_through_the_daemon`: capture, read, edit with
  `baseSeq`, update, list, delete over the daemon socket; a `page_changed` event arrives per write.
- Loopback route test (in `bridge-core`, real `tiny_http` on an ephemeral port): token checks,
  read-only refusal, create then read.
- Protocol gates: `bridge-protocol` tests (`tsgen::checked_in_artifacts_match_the_contract`,
  `params_types_are_named_after_their_method`, strictness), `src-tauri` registry-handler 1:1,
  bridge-deck command signature check.

## Smoke Tests
- `cargo test -p bridge-core notes::` green.
- `bun run test` green (sidecar node tests, vitest, cargo workspace).
- `bun run build` green.
- Regenerating protocol artifacts produces no diff.

## E2E Tests
N/A in this branch: the UI journeys are #867's contract. Live-model behavior is covered by
ignored evals instead:
- `BRIDGE_NOTES_EVAL=1`: Structure eval (about 20 labeled notes) and Turn into tasks eval (about
  12 labeled pages). Report kind/routing accuracy, item recall, task recall/precision, invented
  task rate (0 after guards), guard rejection rate.
- `BRIDGE_NOTES_TOOLS_EVAL=1`: about 15 person-typed requests; score the first notes tool call.

## Manual / cURL Tests
- Build, run `bridged --data-dir /tmp/nb` (short path), then:
  `curl -s -o /dev/null -w '%{http_code}' -X POST 127.0.0.1:4317/notes-tool/x/notes_search -H 'x-bridge-notes-auth: wrong' -d '{}'`
  -> `401`.
- In the app (or `tauri dev`): open a Full-access Claude chat, ask "write a note page called
  Notes smoke test with two bullet points". Then `sqlite3 <data_dir>/notes.db "select title,
  state from pages"` shows it, and `<data_dir>/notes/notes-smoke-test-*.md` exists with the
  bullets.
- Edit that `.md` file in an editor; within 5 s `read_page` (or the next tool read) shows the edit.
