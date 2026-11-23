# feat/import-t3code: Test Contract

Issue #1058 (T3 Code), on the worktree adoption from #1056. A T3 Code user moves to Bridge with their threads as chats and their worktrees as Bridge workspaces, left where they are on disk.

Facts this is built on, researched without opening T3 Code's window:
- T3 Code's source at `pingdotgg/t3code@ccc1feb15`, migration head 102.
- The installed `t3` CLI (0.1.11-nightly.20261120), run headless (`t3 serve --base-dir <sandbox>`) to create real projects, worktrees and Claude, Codex and subagent threads in a throwaway folder.
- A copy of a real old store (migration 43), upgraded by that build.
- Schema fixtures rebuilt with T3 Code's own migrations at 13 heads (`testing/fixtures/import/t3code/`, `schemaDump.test.ts`).

Structure and counts only.

- T3 Code keeps its data under `$T3CODE_HOME`, else `~/.t3`.
  - The live database is `userdata/statev2.sqlite`.
  - Builds before the v2 cutover used `userdata/state.sqlite`.
  - On its first v2 start T3 Code makes `statev2.sqlite` as a full copy of `state.sqlite` and never touches the old file again. So one file is read: `statev2.sqlite` when present, else `state.sqlite`.
- Migration ids are in `effect_sql_migrations`.
  - 1 to 54 are the v1 era.
  - 55 on add the `orchestration_v2_*` tables.
  - 63 drops v1 tables nothing reads.
- **Projects:** `projection_projects` (`workspace_root`, `title`, `scripts_json`, `deleted_at`), in both eras.
- **Threads, v2:** `orchestration_v2_projection_threads`.
  - Columns: `title`, `archived_at`, `deleted_at`, `active_provider_thread_id`.
  - `payload_json` holds `branch`, `worktreePath`, `pinnedAt`, `modelSelection.model`, `lineage.relationshipToParent` (`subagent`, `fork` or none) and `historyOrigin` (`native` or `v1_import`).
- **Threads, v1:** `projection_threads` (`title`, `branch`, `worktree_path`, `archived_at`, `deleted_at`, `pinned_at`, `model_selection_json`).
- **The native session:**
  - v2: the thread's active row in `orchestration_v2_projection_provider_threads`, at `payload_json.nativeThreadRef` (`driver`, `nativeId`). The id is the Claude session id, the Codex thread id or the OpenCode session id.
  - v1, and threads upgraded from v1: `provider_session_runtime.resume_cursor_json`. Claude uses `resume` (or `sessionId`); Codex uses `threadId`.
- **Messages:** `orchestration_v2_projection_messages` (`role`, `payload_json.text`), and `projection_thread_messages` (`role`, `text`) for v1 and for upgraded threads whose transcript T3 has not copied yet.
- **Worktrees:** `<base>/worktrees/<repo>/<branch slug>-<hash>`, recorded on the thread. A thread without one ran in the project folder, the main checkout.
- **Never read:**
  - files: `secrets/`, `settings.json`, `client-settings.json`, `clerk-tokens.json`, `environment-id`;
  - tables: `auth_*`, `scheduled_task*`, `peer_environment_links`, `issue_tracker_bindings`, and the event and outbox tables.

## Functional Behavior

- **Detection:**
  - T3 Code is found when either database exists.
  - It counts chats (threads not deleted, not subagents), projects, and workspaces (worktrees of threads not archived).
- **The version gate:**
  - The database is read when its newest migration is between 5 and 102.
  - It fails closed, saying the version it saw, when the database is newer or older, is missing a column Bridge reads, is not SQLite, or is locked or damaged.
- **Read-only access:**
  - The database is opened read-only. With no live WAL it is opened immutable, so no `-wal` or `-shm` file is created.
  - Only named columns of `effect_sql_migrations`, `projection_projects`, `projection_threads`, `projection_thread_messages`, `provider_session_runtime` (thread id, provider and resume cursor only), `orchestration_v2_projection_threads`, `orchestration_v2_projection_provider_threads`, `orchestration_v2_projection_messages` and `orchestration_v2_legacy_imports` are queried.
- **One chat per thread:**
  - A deleted thread, or a thread in a deleted project, is left out.
  - An archived thread comes over archived.
  - A subagent's thread comes over under its parent.
- **The transcript:**
  - When the native session's file is in Bridge's folders for that agent (Claude Code, including account folders; Codex; OpenCode), the chat is read by that agent's own reader. It has full fidelity and is identical to what that agent's import makes, so importing either way gives one chat, never two.
  - Otherwise the chat is T3 Code's own copy of the messages, user and assistant text in order. It keeps the native identity, so it is still one chat.
- **The title:** T3 Code's title is used, unless the chat was renamed in Bridge.
- **Worktrees:**
  - Every worktree a thread recorded is adopted in place (#1056 rules), named by its thread's title.
  - One whose threads are all archived is left out, as archived in T3 Code.
  - The project's scripts are kept with the workspace, never run.
  - Worktrees are adopted before chats are imported, so chats land under them.
- **Not carried:** pins (Bridge has no pinned chats) and pull request links are reported with counts.
- **Re-running** the import brings nothing twice. Chats already imported by the Claude Code or Codex import are "already in Bridge", and T3 Code's title and archived state are applied to them.
- **The Import page:**
  - T3 Code has an Import button.
  - The result shows chats that came over, workspaces adopted, already in Bridge, and what did not come over, with why.

## Unit Tests

- Reader, on every fixture head (5, 17, 23, 36, 43, 48, 54, 55, 63, 76, 87, 95, 102): reads projects, threads, worktrees, archived, pinned and native ids, with the adaptive columns of each era.
- v2 native refs: Claude, Codex and OpenCode drivers; an unknown driver has none.
- An upgraded v2 store: threads from v1 (`v1_import`) get their native id from `provider_session_runtime`; untranscribed ones read their messages from `projection_thread_messages`.
- Gate: newer, older, no ledger, a missing required column, not SQLite, an empty file.
- Deny-list: every statement the reader runs names only allowed tables. Canaries in `auth_sessions` and `scheduled_tasks` never appear in anything Bridge builds.
- Read-only: no file is created next to a closed database; a live WAL is read through.
- Deleted threads and threads of deleted projects are left out; subagents carry their parent.
- Tool copy: user and assistant text in order; empty and streaming messages are skipped; the native ref is kept.
- Job, against a real fake home: a Claude thread with its transcript imports natively; one whose transcript is gone imports as T3 Code's copy; a Codex thread imports natively; the worktree is adopted and the chats land under it; a second run imports nothing; a Claude import after it brings nothing twice.

## Fuzz and Mutation

- proptest:
  - random rows on every real schema read without panicking;
  - mutated schemas (dropped or renamed columns and tables, a bumped version) fail closed or read correctly against an independent oracle;
  - random thread sets: every live thread maps to exactly one chat, deleted ones to none, and each worktree is a candidate at most once.
- cargo-mutants on `t3code_import.rs` and its store: every surviving mutant is either killed by a new test or shown equivalent.

## E2E

- Fake user (CDP) on a fake home holding a real T3 Code v2 store made by the real `t3` CLI. It clicks Import on T3 Code, waits for the job, and checks:
  - the chats are in the sidebar under the adopted workspace, with T3 Code's titles;
  - the tool-copy chat opens with its messages;
  - the worktree folder is untouched.

## Evals

- Import fidelity: on the sandbox store, every thread's chat in Bridge has the same title and the same user and assistant texts, in order, as T3 Code shows for it (`t3 thread show`).
