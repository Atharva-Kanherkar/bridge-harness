# feat/import-superset: Test Contract

Issue #1060 (Superset, which now calls itself Paradev), on the worktree adoption from #1056 and the T3 Code and Orca imports (#1097, #1127). A Superset user moves to Bridge with:
- their workspaces as Bridge workspaces, left where they are on disk;
- their chats and terminal agent sessions as chats;
- their own agent configurations as inert setup, with secret values never read.

Facts this is built on:
- Superset's source at `superset-sh/superset@17bbe650`.
- The released Paradev CLI 2.0.11 (its checksum matches the release). Its host service ran headless under `env -i`, with a throwaway HOME and app home. It ran offline with a dummy organization and token, as Superset's own sandbox build boots it. Through its own CLI and API it made:
  - a project with two worktree workspaces (one tagged) and a project-less session workspace;
  - an archived workspace;
  - OpenCode chats (one over two turns, one in the session workspace);
  - a Claude Code chat that never started;
  - OpenCode terminal agents, three that failed to start and one that ran;
  - an agent configuration with an API key.
- Schemas rebuilt with Superset's own drizzle migrations at 8 `host.db` heads and 5 `chat.db` heads (`testing/fixtures/import/superset/`, `rebuild.py`). The newest heads match what the real host created, column for column.

Structure and counts only.

- **App home:**
  - `$SUPERSET_HOME_DIR`, else `$PARADEV_HOME_DIR`, else `~/.paradev`.
  - Older installs used `~/.superset`. Superset moves it to `~/.paradev` and leaves `~/.superset` as a symlink; if the move fails it keeps using `~/.superset`.
  - Development homes (`~/.paradev-<name>`) are not read.
- **Organizations:**
  - Each one has `host/<organization id>/` holding `host.db`.
  - `chat.db` sits beside it, made on the first chat.
  - `host/standalone/` holds no database.
- **Versions:** both databases are drizzle databases. The version is the newest `__drizzle_migrations.created_at`, the migration's journal time.
  - `host.db`: 0000 (1773188543980) to 0038 (1792467855578).
  - `chat.db`: 0000 (1785962456515) to 0005 (1794194559956).
- **Projects:** `projects` (`id`, `repo_path`, `name`, `worktree_base_dir`, `deleted_at`).
- **Workspaces:** `workspaces` (`id`, `project_id`, `worktree_path`, `branch`, `name`, `type`, `archived_at`).
  - `type` is `worktree` (its own checkout), `local` (the project's checkout) or `session` (a project-less scratch folder in `<home>/sessions/`).
  - Archiving keeps the row and removes the folder.
  - New worktrees go to `<home>/worktrees/<project>/<branch>`.
- **Chats:** `chat.db` `chat_sessions_local` (`session_id`, `scope_id` = the workspace, `harness`, `harness_session_id`, `epoch`, `title`, `closed_at`).
  - `harness` is `claude-acp`, `codex-acp`, `opencode-acp`, `pi-acp`, `pdev-acp` or `gemini-acp`; older builds used `claude-code` and `codex`.
  - `harness_session_id` is the agent's own session id; it is empty for a chat that never started.
  - The host does not title chats (`title` is empty in practice).
- **Superset's copy of a chat:** `chat_journal` (`session_id`, `epoch`, `seq`, `event_json`) at the session's current `epoch`.
  - An event is `session`, `turn` or `item`. An item is journaled again each time it changes: the latest body counts, at the position it first appeared.
  - Items: `user_message` (`content[]` text parts; `hidden` parts and `<superset-context>` blocks are not the person's words), `agent_message` (`text`), `reasoning` (`text`), `tool_call` (`toolName`, `title`, `status`, `rawInput`, `content[]`), plus `plan`, `approval_request` and `notice`.
  - Items with `parentItemId` belong to a subagent.
- **Terminal agents:** `terminal_agent_bindings` (`terminal_id`, `workspace_id`, `agent_id`, `agent_session_id`, `transcript_path`).
  - There is one row per terminal; a new session in the same terminal replaces it.
  - `agent_session_id` is empty when the agent never reported one.
  - The transcript is the agent's own (Claude Code's `transcript_path`, Codex's rollout, OpenCode's store).
- **Agent configurations:** `host_agent_configs` (`id`, `preset_id`, `label`, `command`, `args_json`, `env_json`).
  - Superset seeds one per built-in agent, with no environment.
  - `env_json` holds API keys in plain text.
- **Tags and pull requests:** `workspace_tags` and `workspace_pull_requests`.
- **Project config:** `.paradev/` or the older `.superset/` in the repository.
  - `config.json`: `setup`, `teardown` and `run`, each a list of commands.
  - `config.toml`: `[setup] run`, `[archive] run`.
- **Never read:**
  - files: `auth-token.enc`, `config.json` in the app home (OAuth and API key), each organization's `manifest.json` (host token), `terminal-host.token`, `cloud-workspace-tickets.json`, `plugin-mcp-sessions.json`, `state/`, `agent/`;
  - `host_agent_configs.env_json` values (only its key names, listed by SQLite);
  - `mobile_*`, `pull_requests`, `chat_sync_state` and `chat_selections`;
  - the v1 desktop `local.db`.

## Functional Behavior

- **Detection:**
  - Superset is found when its app home has an organization folder with a `host.db`.
  - It counts:
    - chats: chats and terminal agent sessions with an agent Bridge runs, once per native session;
    - projects: projects not deleted;
    - workspaces: worktree workspaces not archived.
  - `~/.paradev` and a `~/.superset` symlink to it are one home, counted once.
- **The version gate:**
  - `host.db` is read from 0000 to 0038 and `chat.db` from 0000 to 0005.
  - It fails closed, saying the version it saw, when either is newer or older, has no migration journal, is missing a column Bridge reads, is not SQLite, or is damaged.
- **Read-only access:**
  - Databases are opened read-only. With no live WAL they are opened immutable, so no `-wal` or `-shm` file is created.
  - A WAL Superset left behind is read through. Superset's host stops without checkpointing, so after it quits most of the data is still in the WAL.
  - Only named columns of allowed tables are queried.
- **One chat per native session:**
  - Each chat and each terminal agent session with a session id is one chat.
  - A chat and a terminal session naming the same session, or either naming a session another import brought, is still one chat.
- **Its folder:** the workspace's `worktree_path`.
- **The transcript:**
  - When the native session's file is in Bridge's folders for that agent (Claude Code, including the folder a terminal session's `transcript_path` names and Superset's default Claude Code folder; Codex, including Superset's default Codex home; OpenCode), the chat is read by that agent's own reader.
  - Otherwise a chat is Superset's own copy, which keeps the native identity:
    - the person's messages, the agent's messages, reasoning, and tool calls with their input and output, in order;
    - not subagent items, plans, approvals or notices.
  - A terminal session whose transcript is gone has no Superset copy; it is counted.
  - A chat that never started and has nothing to show is left out.
- **Left out and counted:**
  - chats and terminal sessions with an agent Bridge does not run, by agent;
  - terminal agents that never reported a session;
  - chats in a workspace of a deleted project.
- **Archived:** a chat in an archived workspace comes over archived.
- **The title:** Superset's title when it has one, else Bridge's own (the first thing the person said). Never over a chat renamed in Bridge.
- **Workspaces:**
  - Every worktree workspace is adopted in place under the #1056 rules, named by Superset's name, with the project's setup and archive scripts kept, never run.
  - Archived ones are left out, as archived in Superset.
  - `local` workspaces are the project's checkout, a project, not a workspace. A workspace at its project's own folder is never adopted, whatever its type: before 0017 workspaces had no type.
  - `session` workspaces are scratch folders, not adopted.
  - Workspaces are adopted before chats, so chats land under them.
- **Agent configurations:** each one you made or gave an environment comes over as an inert, disabled setup candidate: its label, agent, command, arguments and the names of its environment variables, never their values. Superset's untouched built-in configs are not candidates.
- **Not carried, with counts:** workspace tags and pull request links.
- **Re-running** the import brings nothing twice. Chats already imported by the Claude Code, Codex or OpenCode import are "already in Bridge".
- **The Import page:**
  - Superset has an Import button.
  - The result shows chats that came over, workspaces adopted, already in Bridge, and what did not come over, with why.

## Unit Tests

- **Reader on every fixture head** (host 0, 6, 17, 20, 26, 33, 36, 38; chat 0, 1, 3, 4, 5): projects, workspaces with their type and archived state, chats with native ids, terminal bindings, with the columns each head has.
- **Harness and agent ids:** every ACP and legacy harness; terminal agent ids; unknown ones have no harness.
- **The journal:** latest body by id at first position; hidden parts and `<superset-context>` stripped; subagent items, plans and notices skipped; another epoch skipped.
- **Gate:**
  - a newer and an older journal on either database;
  - no journal;
  - a missing required column;
  - not SQLite, an empty file.
- **Deny-list:**
  - Every statement names only allowed tables. `env_json` is only read through `json_each` keys.
  - Canaries in `env_json`, `manifest.json`, `config.json`, `auth-token.enc` and the mobile tables never appear in anything Bridge builds.
- **Read-only:** no file is created next to a closed database; a WAL left behind is read through.
- **Homes:** `SUPERSET_HOME_DIR` wins over `PARADEV_HOME_DIR`; `~/.superset` as a symlink and `~/.paradev` are one home; `~/.superset` alone is read; organizations without a `host.db` are skipped.
- **Project config:** `config.json` and `config.toml` scripts, `.paradev` over `.superset`, garbage.
- **Agent configurations:** a config with an environment and a custom one become candidates with key names only; seeded built-ins do not.
- **Job, against a fake home:**
  - an OpenCode chat whose session is in OpenCode's store imports natively;
  - a Claude Code chat whose transcript is gone imports as Superset's copy;
  - a terminal agent session imports from its Claude Code transcript;
  - the worktree is adopted and the chats land under it;
  - an archived workspace is left out and its chat is archived;
  - a second run imports nothing;
  - an OpenCode import after it brings nothing twice.

## Fuzz and Mutation

- **proptest:**
  - random rows on every real schema head read without panicking or leaking;
  - mutated schemas (dropped or renamed columns and tables, shifted journals) fail closed or read correctly against an independent oracle;
  - random chat and binding sets: every live native session comes over exactly one way, the rest never, and each worktree is a candidate at most once;
  - journal events and project configs of any shape never panic.
- **cargo-mutants** on `superset_import.rs` and its store: every surviving mutant is killed by a new test or shown equivalent. The same goes for the Orca import's remaining mutants (#1127), whose fixes land here.

## E2E

- A fake user (CDP) on a fake home whose app home is built on the newest schemas, with rows shaped as the real host wrote them:
  - a project with a worktree workspace holding uncommitted work, and an archived one;
  - an OpenCode chat on the session the OpenCode import brings, and a chat whose session is gone;
  - a terminal session on an agent Bridge does not run;
  - an agent configuration with a canary key.

  It clicks Import on Superset, waits for the job, and checks:
  - Superset is found with 2 chats and 1 workspace;
  - the copy and the agent configuration came over, and the OpenCode chat was already in Bridge;
  - the page says the configuration came over switched off;
  - what did not come over names the archived workspace and the Amp terminal;
  - the workspace keeps Superset's name, folder and branch, with the chat under it, and the archived one is not a workspace;
  - the copied chat opens badged as from Superset, both sides in order;
  - no canary shows anywhere;
  - the folder and its uncommitted work are untouched.
- Every tool Bridge knows is now found in the fake home, each with its own Import button.

## Evals

- Import fidelity (`scripts/e2e/import-superset-fidelity.mjs`): on the real sandbox app home the 2.0.11 host wrote, every chat's user and assistant texts, in order, match what Superset itself shows for it (`superset sessions read`, captured while the host ran). The terminal session matches what OpenCode's own export shows (`opencode session export`).
