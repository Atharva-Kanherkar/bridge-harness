# feat/import-codex: Test Contract

Issue #1054, chats first. Codex chats come over with the one Import button, as Claude Code's do: in the background, with progress, with their commands, edits, MCP calls and replies, and continuable afterwards.

Facts this is built on, from a real `~/.codex` (codex-cli 0.159.3; counts only):

- 313 rollouts, 327 MB.
- Every rollout starts with `session_meta`, and every one has `event_msg` `item_completed` events. Those are what Codex showed the person: `UserMessage`, `AgentMessage`, `Reasoning`, `CommandExecution`, `FileChange`, `McpToolCall`, `ImageView`, `Extension` (web search), `SubAgentActivity` and `ContextCompaction`.
- `response_item` holds the model-level copy. It includes injected `<environment_context>` and AGENTS.md blocks as user messages, and an `exec` custom tool whose input is JavaScript, not a shell command.
- 347 of 352 sessions were started by Bridge itself (`originator: bridge`), and 219 of those ran in the OS temp folder.
- Subagent threads name their parent in `session_meta.source.subagent.thread_spawn.parent_thread_id`. Titles are in `session_index.jsonl`.

## Functional Behavior

- The Import page offers Import for Codex when a Codex home is found (`CODEX_HOME`, else `~/.codex`). It runs the same background job as Claude Code: progress, the transfer animation, the finish notice and the summary.
- Each rollout under `sessions/**/rollout-*.jsonl` and `archived_sessions/` becomes a chat:
  - title from `session_index.jsonl`, else derived from the first message;
  - cwd, model and effort;
  - an archived thread imports as archived.
- A chat's history is built from `item_completed` events, in order:
  - `UserMessage` → your message;
  - `AgentMessage` → Codex's reply;
  - `Reasoning` with summary text → thinking;
  - `CommandExecution` → a shell command, with its output and failed when the exit code is not 0;
  - `FileChange` → an edit naming the files;
  - `McpToolCall` → an MCP call (`mcp__server__tool`);
  - `ImageView` and web search → tool rows;
  - `compacted` → a compaction point with Codex's summary.
  Records Bridge does not carry are counted; known bookkeeping records are not losses.
- A subagent thread becomes a child chat of its parent, even when the parent was read in an earlier batch. Its native key finds the parent in the store.
- Threads Bridge itself ran are recognised as already in Bridge, by the Codex thread id Bridge recorded, and are not imported again. Threads that ran in the OS temp folder are background runs: counted and left out.
- A thread listed in `external_agent_session_imports.json` whose source still exists, and in which nothing was typed in Codex, is left out; that conversation comes over from its own tool. Otherwise it imports.
- `auth.json`, `logs_*.sqlite`, `mcp-oauth-locks/` and the other private stores are never read.
- Continuing an imported Codex chat (#1061) resumes the Codex thread natively when:
  - the rollout is still in a Codex home Bridge signs in with;
  - the folder exists;
  - no other chat holds the thread.

  Otherwise it starts with the history.
- Memories stay with Codex, which reads its own. Setup (AGENTS.md, prompts, skills) is out of this PR.

## Unit Tests

- Rollout reader, one fixture per item type: user and agent messages, empty and non-empty reasoning, a command that passed and one that failed, a file change, an MCP call, an image view, a web search, a compaction, an unknown record type, and an over-long line.
- Title from `session_index.jsonl`, and the derived fallback; model and effort from `turn_context`; archived threads.
- A subagent's parent key; Bridge's own and temp-folder threads; the external-import rule.
- `job_work` lists rollouts in order, leaves background runs out, and never opens a file it does not need.

## Integration / Functional Tests

- A fake Codex home imports through the job: chats with their tool rows, a child chat under its parent across two batches, archived, background left out, and nothing imported twice on a second run.
- A Bridge-run thread (`sessions.provider_session_id` = the thread id) is counted as already in Bridge.
- Continuation plan for Codex: native when everything holds; each history reason otherwise.
- Detection marks Codex supported, with counts.

## Smoke Tests

- `bun run build`
- `cargo test -p bridge-core --lib codex_import import_job import_continue`
- `cargo test -p bridge-protocol`, `cargo test -p bridge-deck --lib`

## E2E Tests

- Real Codex CLI: make two real sessions with `codex exec` in a scratch folder, each told a code word. Import them through the app against a real `bridged`. Continue the first: the reply comes from the same Codex thread. Delete the second's rollout and continue it: a new chat answers from the history. The imported chats are unchanged.
- Real store: the job over this Mac's `~/.codex`, into a scratch database, with aggregate numbers only.

## Manual Tests

- Import, then Codex, on a Mac with Codex history: progress runs, the chats appear with their commands and edits, and Bridge's own chats are not duplicated.
