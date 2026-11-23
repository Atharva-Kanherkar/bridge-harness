# feat/import-orca: Test Contract

Issue #1059 (Orca), on the worktree adoption from #1056 and the T3 Code import (#1097). An Orca user moves to Bridge with their worktrees as Bridge workspaces, left where they are on disk, their chats, and their multi-agent runs as orchestrator and worker chats.

Facts this is built on:
- Orca's source at `stablyai/orca@1f4e0b30`.
- The released Orca 1.4.271 (signed and notarized), run headless (`orca serve` on its Node runtime) under `env -i` with a throwaway HOME and data folder. Through its own CLI it created:
  - a repository with two worktrees;
  - OpenCode chats (one renamed, one a coordinator);
  - a Claude Code chat that never started;
  - two multi-agent runs, one led by that chat and one led by a plain terminal, each with a terminal worker.
- Schema fixtures dumped from the databases that build wrote (`testing/fixtures/import/orca/1.4.271/`).

Structure and counts only.

- **Data folder:** `$ORCA_USER_DATA_PATH`, else `~/Library/Application Support/orca`.
  - Development builds (`orca-dev`) are not read.
- **Profiles:**
  - `orca-profile-index.json` lists them; most installs have only `local-default`.
  - Each profile's state is `profiles/<id>/profile-state.db`, at `PRAGMA user_version` 3.
  - `profile_state_documents` holds one JSON document per key (`domain`). Keyed maps such as `worktreeMeta` are also split into `profile_state_document_parts`. The parts are current only when `profile_state_document_parts_meta.legacy_revision` equals the document's `revision`.
  - Before 1.4.214, state was `profiles/<id>/orca-data.json`. That file also holds settings and keys, so it is never read.
- **Repositories:** `repos[]` (`id`, `path`, `displayName`, `kind`, `connectionId` for SSH).
- **Worktrees:**
  - A worktree's id is `<repo id>::<absolute path>`, with an optional `::workspace:<uuid>` suffix.
  - `worktreeMeta[<id>]` holds `displayName`, `isArchived`, `comment` and `baseRef`. A missing field means its default.
  - The repository's main checkout is `<repo id>::<repo path>`.
  - Orca makes new worktrees in `~/orca/workspaces/<repo>/<name>`.
  - Folder workspaces (`folderWorkspaces[]`: `id`, `folderPath`) are folders, not worktrees.
- **Chats:**
  - They live in `agent-session-domain.db` at the data folder's root, shared by every profile, at `user_version` 2.
  - `agent_session_records.record_json` holds:
    - `sessionId` and `provider` (`claude`, `codex`, `opencode`, …);
    - `location.workspaceId` (a worktree id, `folder:<id>`, or `global-floating-terminal`) and `location.executionHostId` (`local`, or another machine);
    - `launchDirectory`, `conversationName`, `conversationNameSource` (`user` when renamed), and `createdAt`/`updatedAt` in ms;
    - `accountHome` (`variable` and `path` for Claude Code and Codex; `kind: opencode` for OpenCode);
    - `providerHandleChain`.
  - The last link of `providerHandleChain` is the native session:
    - Claude Code: `{provider: claude, sessionId}`;
    - Codex: `{provider: codex, threadId}`;
    - OpenCode and other ACP agents: `{transport: acp, agent, nativeId}`.
  - A chat that never started has no link.
  - Deleting a chat removes its record and leaves `chat.deleted:<id>` in `agent_session_store_meta`.
  - Orca's own copy is `journal_items` at the chat's current `journal_chat_state.epoch`, where `hidden = 0`, ordered by `sequence, sequence_index, ordinal`. `payload_json.body` is one of:
    - `message` with role `user`, `assistant` or `reasoning`, and text blocks;
    - `tool-call` with `name`, `input`, `state`, and `output.head`;
    - `status`, `turn` or `agent-start`, which are not conversation.
  - Rows carrying an `agentId` are a subagent's.
  - Before the domain database, chats were in `agent-session-journal.db`. Orca converts it on its next start and keeps it.
- **Multi-agent runs:** `orchestration.db` at the root (`user_version` 50 in 1.4.271, 51 in the source).
  - `runs` (`id`, `objective`, `coordinator_orca_session_id`, `coordinator_handle`). A run led by a chat names it; a run led by a terminal names only the terminal.
  - `dispatch_contexts` (`run_id`, `assignee_orca_session_id`): a worker that runs as a chat.
  - `worker_terminal_resources` (`origin_dispatch_id`, `resume_session`): a worker in a terminal. Its `resume_session` (`agent`, `providerSession.id`) is captured from Claude Code and Codex hooks, and is empty for OpenCode.
- **Project config:** `<repo>/orca.yaml` `scripts.setup` and `scripts.archive`. Orca has no run script.
- **Never read:**
  - in the data folder: `orca-data.json`, `orca-runtime.json`, `orca-e2ee-keypair.json`, `orca-devices.json`, `orca-secret-protection.json`, `connector-host-key.json`, `agent-session-authority.key`, `agent-hooks/`, `claude-profiles/`, `claude-runtime-auth/`, `codex-accounts/`, `codex-runtime-home/`, `antigravity-accounts/`, `managed-data-accounts/`, `plugins-data/`, `Partitions/`, `google-setup-keys/` and `bot-tool-sign-in/`;
  - in each profile: `account-session.json.enc` and `browser-passwords/`;
  - in `profile-state.db`: every document but `schemaVersion`, `repos`, `projects`, `folderWorkspaces` and `worktreeMeta`. Never `settings`, `sshTargets`, `ui`, `workspaceSession`, or any other key;
  - `~/.orca/`;
  - Claude Code transcripts under an account folder Orca names (`accountHome.path`): only `projects/<folder>/<session>.jsonl`, the same files the Claude Code import reads. Never its credentials.
- **Read only for what is needed:**
  - `orca-profile-index.json` is read for profile ids only.
  - A chat record and a terminal worker's `resume_session` also hold leases, account routes and a worker's environment (which can carry API keys). Bridge names the fields it wants and SQLite extracts them (`json_extract`), so the rest never leaves the database.

## Functional Behavior

- **Detection:**
  - Orca is found when its chat database or profile index exists.
  - It counts:
    - chats: local, with an agent Bridge runs, not deleted;
    - projects: local repositories in any profile;
    - workspaces: worktrees not archived and not a main checkout.
- **The version gate:**
  - The chat database is read at `user_version` 1 to 2, each profile's state at 1 to 3, and `schemaVersion` at 1.
  - It fails closed, saying the version it saw, when either is newer or older, is missing a column Bridge reads, is not SQLite, or is damaged.
  - With only the old `agent-session-journal.db`, it asks to open Orca once and import again.
  - `orchestration.db` is read when the columns Bridge reads exist, at any version. Otherwise runs are not linked, and that is reported.
- **Read-only access:**
  - Databases are opened read-only. With no live WAL they are opened immutable, so no `-wal` or `-shm` file is created. A running Orca's live WAL is read through.
  - Only named columns of allowed tables are queried. Profile documents are selected by name. The whole `record_json` and `resume_session` columns are never selected.
- **One chat per record:**
  - Deleted chats are left out.
  - Chats on another machine (an SSH or WSL host) are left out and counted.
  - Chats with an agent Bridge does not run (anything but Claude Code, Codex and OpenCode) are left out and counted by agent.
  - A chat whose worktree Orca archived comes over archived.
- **Its folder:** the worktree's path, else the folder workspace's folder, else `launchDirectory`.
- **The transcript:**
  - When the native session's file is in Bridge's folders for that agent, the chat is read by that agent's own reader. That covers Claude Code (including the account folder Orca launched it with), Codex and OpenCode. Its fidelity is full and it is the same chat that agent's import makes, so importing either way gives one chat.
  - Otherwise the chat is Orca's own copy, which keeps the native identity:
    - user and assistant text in order;
    - reasoning;
    - tool calls with their input and output;
    - not subagent rows, status or turn markers.
  - A chat that never started and has nothing to show is left out.
- **The title:** Orca's chat name is used, unless the chat was renamed in Bridge.
- **Multi-agent runs:**
  - A run led by a chat makes that chat the orchestrator. Each worker the run dispatched becomes its child chat, both as a chat and as a Claude Code or Codex terminal whose session Orca captured.
  - This is historical: nothing is delegated again.
  - Workers of a run led by a terminal, and OpenCode terminal workers, stay top-level chats, and are counted.
- **Worktrees:**
  - Every worktree in any profile's `worktreeMeta`, or named by a chat, is adopted in place under the #1056 rules, named by Orca's display name.
  - Main checkouts are projects, not workspaces. SSH repositories are left out.
  - Archived ones are left out, as archived in Orca.
  - `orca.yaml` setup and archive scripts are kept with the workspace, never run.
  - Worktrees are adopted before chats, so chats land under them.
- **Re-running** the import brings nothing twice. Chats already imported by the Claude Code, Codex or OpenCode import are "already in Bridge", and Orca's titles, archived state and run links are applied to them.
- **The Import page:**
  - Orca has an Import button.
  - The result shows chats that came over, workspaces adopted, already in Bridge, and what did not come over, with why.

## Unit Tests

- **Reader on the 1.4.271 schema fixture:**
  - profiles, repositories, worktrees with display names and archived state, folder workspaces;
  - parts taking over the document only when current;
  - chats with folders, native refs for every handle form, a chat with no handle, and deleted tombstones.
- **Native refs:** Claude Code `sessionId`, Codex `threadId`, ACP OpenCode `nativeId`; an unknown agent has none; the last link wins.
- **Worktree ids:** a suffix, `::` in a path, a folder id, a floating terminal, garbage.
- **Gate:**
  - a newer and an older chat database;
  - a newer profile state;
  - no records table;
  - a missing required column;
  - not SQLite, an empty file;
  - only the old journal.
- **Deny-list:**
  - Every statement names only allowed tables.
  - Canaries in `settings`, `sshTargets` and `workspaceSession` documents never appear in anything Bridge builds.
  - The listed secret files are planted unreadable (mode 000); the read still succeeds, and they are still unreadable after it.
- **Read-only:** no file is created next to a closed database; a live WAL is read through.
- **Tool copy:** messages, reasoning and tool calls in order; hidden rows, old epochs, subagent rows and status rows are skipped; the native ref is kept.
- **Runs:**
  - a chat-led run links its chat and terminal workers;
  - a terminal-led run links nothing and is counted;
  - an OpenCode terminal worker is counted;
  - an orchestration database missing a column is reported.
- **`orca.yaml`:** block, folded and single-line scripts, no `scripts`, other keys, a run script ignored, garbage.
- **Job, against a fake home:**
  - an OpenCode chat whose session is in OpenCode's store imports natively;
  - a Claude Code chat whose transcript is gone imports as Orca's copy;
  - worktrees are adopted, and the chats land under them;
  - a chat-led run's workers end up under its orchestrator;
  - a second run imports nothing;
  - an OpenCode import after it brings nothing twice.

## Fuzz and Mutation

- **proptest:**
  - random records, documents and journal rows on the real schemas read without panicking;
  - mutated schemas (dropped or renamed columns and tables, bumped versions) fail closed or read correctly against an independent oracle;
  - random record sets: every live record maps to at most one chat, and each worktree is a candidate at most once;
  - worktree ids and `orca.yaml` never panic and round-trip known forms.
- **cargo-mutants** on `orca_import.rs` and its store: every surviving mutant is killed by a new test or shown equivalent.

## E2E

- A fake user (CDP) on a fake home whose Orca data folder is built on the 1.4.271 schemas, with rows shaped as that build wrote them. A lead chat ran on the OpenCode session the OpenCode import brings; its worker ran as an Orca chat in an Orca worktree, on a session OpenCode no longer has; there is a Pi chat, a deleted chat and an OpenCode terminal worker without a session. It clicks Import on Orca, waits for the job, and checks:
  - Orca is found with 2 chats and 1 workspace; one came over, one was already in Bridge, the worktree is a workspace;
  - what did not come over names the Pi chat and the unlinked worker;
  - the workspace keeps Orca's name, folder and branch, and the worker's chat is in it, under the chat that led its run;
  - the Pi and deleted chats did not come over, and no canary shows;
  - the worker's chat opens from Orca's copy, badged as imported from Orca, both sides in order;
  - the worktree and its uncommitted work are untouched.
- The fake user follows Settings' current layout (Import is its own page under Data; Settings takes the sidebar's place, so it goes back to the app to open a chat).

## Evals

- Import fidelity (`scripts/e2e/import-orca-fidelity.mjs`): on the real sandbox data folder Orca 1.4.271 wrote, with its OpenCode store, every chat's title and its user and assistant texts, in order, match what Orca shows for it (`orca agent read <id> --json`, and the titles `orca agent list` showed, captured while Orca ran).
