# feat/import-opencode: Test Contract

Issue #1055, chats first, plus the imported-subagent gap left by #1053/#1054. OpenCode chats come over through OpenCode's own section on the Import page, the way Claude Code's and Codex's do: in the background, with progress, with their commands, edits, subagents and replies, and continuable afterwards. Imported subagent chats, from any tool, can be opened from the sidebar.

Facts this is built on, from a real `~/.local/share/opencode` (opencode 2.1.1; counts only):

- One SQLite store, `opencode.db` (561 MB, WAL). Chats are rows, not files.
- Two generations of tables in the same store. 2.x writes `session_v2` with `session_message` rows (`user`, `assistant`, `synthetic`, `system`, `compaction`, `idle`, `location-switched`). 1.x wrote `session`, `message` and `part`; its 60 sessions were copied into `session_v2` when 2.x upgraded the store.
- An assistant row's `content` holds `text`, `reasoning` and `tool` items. Tools seen: `shell`/`bash`, `read`, `edit`, `write`, `patch`, `grep`, `glob`, `webfetch`, `websearch`, `todowrite`, `task`, `subagent`, `execute`, `question`, `skill`, and MCP tools named `<server>_<tool>`. A tool has `state.status` (`completed`, `error`), `state.input`, `state.content` and `state.metadata` (`exit` for commands, `diff` for edits).
- A subagent session names its parent in `parent_id` (132 of 545). `synthetic` rows carry a background subagent's or command's result back to the parent, or OpenCode's own "continue" nudge.
- 106 sessions were run by Bridge (`sessions.provider_session_id`), 119 ran in the OS temp folder, and 151 ran in Bridge's title scratch folder (`<Bridge data>/chats/title-runs`).
- The same store holds `account`, `credential` and `control_account` tables with tokens.

## Functional Behavior

- OpenCode is offered when its store is found: `$XDG_DATA_HOME/opencode/opencode.db`, else `~/.local/share/opencode/opencode.db`. Its section shows OpenCode's mark, the chats and projects it holds, and its own Import button. It runs the same background job as Claude Code and Codex: progress, the transfer animation with OpenCode's tile, the finish notice naming OpenCode, and its own last result.
- The store is opened read-only, without blocking OpenCode while it runs. Only the session, message and part tables are read. `account`, `credential`, `control_account`, `auth.json` and every other private table or file are never read.
- Each OpenCode session becomes a chat:
  - title, folder, model and effort (the model's variant);
  - an archived session imports as archived;
  - 2.x sessions from `session_v2`; a 1.x-only store from `session`, `message` and `part`; a session in both is read once, from `session_v2`.
- A chat's history, in order:
  - your message, with its inline images;
  - OpenCode's reply and its reasoning;
  - each tool as a row: commands with their output, failed when the exit code is not 0 or the tool errored; reads, edits (with the diff), writes and patches naming the file; searches, fetches, todos and MCP calls by name;
  - `task` and `subagent` calls as delegations, with the description, the prompt and the agent;
  - a background subagent's or command's result as the row it finished;
  - a compaction as a compaction point with OpenCode's summary.
  Records Bridge does not carry are counted; known bookkeeping (`system`, `idle`, `location-switched`, step markers, OpenCode's "continue" nudges) is not a loss.
- A subagent session becomes a child chat of its parent, even when the parent was read in an earlier batch.
- Sessions Bridge itself ran are recognised as already in Bridge, by the OpenCode session id Bridge recorded, and are not imported again. Sessions that ran in the OS temp folder or in Bridge's title scratch folder are background runs: counted and left out.
- Importing again brings nothing over twice.
- Continuing an imported OpenCode chat (#1061) resumes the OpenCode session natively when:
  - the session is still in an OpenCode store Bridge signs in with (your own, or an OpenCode account's data folder in Settings);
  - the folder exists;
  - no other chat holds the session.

  Otherwise it starts with the history.
- Memories and setup stay with OpenCode.

### Imported subagent chats in the sidebar

- An imported subagent chat is listed under the chat it belongs to, whatever tool it came from. A chat with imported subagents shows how many it has and unfolds to list them. They start folded.
- Clicking a subagent opens it like any imported chat: its history, the imported badge, and the continuation notice (a subagent always continues with its history). It never shows the background-worker steering composer.
- The chat the open subagent belongs to unfolds by itself, so the open row is visible.
- Subagent rows follow the sidebar's filter and search through their parent; they are never listed at the top level.

## Unit Tests

- Store reader on a fixture store built with OpenCode's own schema, one fixture per shape:
  - user text and an inline image; a reply; reasoning;
  - a command that passed and one that failed; a tool that errored;
  - read, edit with a diff, write, patch, grep, glob, webfetch, websearch, todowrite, an MCP tool;
  - `task` and `subagent`; a background subagent's `synthetic` result; a background command's result; the "continue" nudge;
  - a compaction; an unknown row type; an unreadable row;
  - model and effort; an archived session.
- The same chat from 1.x `message`/`part` rows; a session in both generations is listed once.
- `job_work` lists sessions in order, parents before children, and leaves temp-folder and title-run sessions out. It never reads a message to do so.
- The credential tables are never queried: the fixture's `credential` and `account` tables hold canaries, and no output contains them.
- A session's native key, and its parent's.

## Integration / Functional Tests

- A fixture store imports through the job: chats with their tool rows, a child chat under its parent across two batches, archived, background left out, and nothing imported twice on a second run.
- A Bridge-run session (`sessions.provider_session_id` = the session id, harness `opencode`) is counted as already in Bridge.
- Continuation plan for OpenCode: native when everything holds; each history reason otherwise, including a store Bridge does not sign in with.
- Detection marks OpenCode supported, with counts of chats and projects, background runs apart, subagents not counted.
- `MoveToBridge.test.tsx`: OpenCode's section has its own button and runs its own import.
- `sidebarChats.test.ts`: imported subagents are grouped under their top chat, nested ones included, ordered by time, and an orphan is listed under nothing.
- `BridgeSidebar.test.tsx`: the count, unfolding, opening a subagent, the open subagent's parent unfolding by itself, and filters keeping subagents with their parent.
- `App` view: an imported subagent opens without the worker steering composer.
- Scripted fake user: imports Claude Code, then OpenCode from its own section; opens an imported subagent from the sidebar.

## Smoke Tests

- `bun run build`
- `cargo test -p bridge-core --lib opencode_import import_job import_continue import_sources`
- `cargo test -p bridge-protocol`, `cargo test -p bridge-deck --lib`

## E2E Tests

- Real OpenCode CLI: make two real sessions with `opencode run` in a scratch folder, each told a code word, in a scratch data folder. Import them through the app against a real `bridged`. Continue the first: the reply comes from the same OpenCode session. Delete the second session from the store and continue it: a new chat answers from the history. The imported chats are unchanged.
- Real store: the job over this Mac's OpenCode store, into a scratch database, with aggregate numbers only. Memory is measured on the test binary run directly.

## Manual Tests

- Import, then OpenCode, on a Mac with OpenCode history: progress runs, the chats appear with their commands and edits, Bridge's own chats are not duplicated, and a chat's subagents unfold under it in the sidebar.
