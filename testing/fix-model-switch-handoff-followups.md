# Model switch handoff, the remaining decisions — Test Contract

Locked before implementation. The second slice of the model-switch context
issue. The first slice (switch summaries are checkpoints, a failed summary
builds no wall, each app's thread is remembered and resumed, the notice and
ledger say what was sent) is merged. This branch does the four decisions it
left open:

- D3: the handoff never depends on the outgoing model. Bridge writes a header
  from stored history without a provider call.
- D5: switching account on the same app carries the thread to the new account
  and resumes it natively.
- D6: a chat can read its own stored history, tool output included, on demand.
  The handoff names the tool and where to start reading.
- D7: the handoff carries tool facts (what ran, which files changed), not only
  chat text.

## Functional Behavior

### F1 (D7). Tool facts in the handoff tail

- Completed tool entries (`command.completed`, `file_change.completed`,
  `tool.completed`) on the active branch become short facts:
  - command: the command's first line, with a `zsh/bash/sh -lc '…'` wrapper
    unwrapped, clipped to 120 chars, plus its status (`exit N`, or `failed`).
  - file change: `edited <path>` for each path (Codex `data.changes[].path`,
    OpenCode `data.state.input.filePath|path`, Claude's started `input.file_path`).
  - other tool: its name plus one target (`file_path`, `path`, `url`,
    `query`, `pattern`, `command`, in that order), clipped.
- A Claude completed entry carries no command or path, so the fact reads them
  from the latest started entry with the same `itemId`.
- Started entries, output deltas and reasoning never make a fact. No fact
  carries tool output.
- Consecutive facts between two conversation entries merge into one
  `tools:` line. A line shows at most 6 facts and at most 480 bytes, then
  `(+N more)`.
  - Amended during implementation: the line names the newest facts of the
    run, in the order they ran, after `(+N earlier)`. The last calls before a
    reply (the test run that passed, the PR it opened) say where the work
    stands. Facts are extracted raw and scrubbed only when shown, so a branch
    of thousands of tool calls costs a scrub only for the lines that fit.
- Every fact goes through the history scrubber, so a token in a command
  line never reaches the handoff.
- Tool lines count against the same tail budget as messages. A branch with no
  tool entries renders exactly as before (existing restoration tests are
  unchanged).
- `carried_entries` and `eligible_entries` still count conversation entries
  only, so the notice's "the latest 40 of 147 messages" keeps its meaning.
- The "since you left" delta for a returned thread also carries the tool
  lines from the other app's turns.

### F2 (D3). A handoff header Bridge builds without a model

- `restoration_handoff` always writes a Bridge facts block when the branch
  has any conversation, with or without a model summary:
  - `goal:` the latest user message, clipped to 400 bytes.
  - `started with:` the first user message, clipped to 400 bytes, only when
    it is a different entry and the tail did not carry it.
  - `files touched:` up to 15 paths, newest first, deduplicated: file-change
    facts, `worker.result.filesChanged`, `artifact.created.path`, and the
    newest checkpoint's `filesTouched`. More than 15 adds `(+N more)`.
  - `links:` up to 8 GitHub or GitLab pull request, merge request or issue
    URLs, newest first, deduplicated, seen in messages, worker results or
    tool commands and output. Query strings are never part of a match.
- A model summary is additive: when a compaction or checkpoint exists, its
  line, decisions and open work come first and the facts follow.
- The facts block is bounded (4 KiB). The whole handoff still respects the
  incoming model's budget and the 96 KiB cap.
- Deterministic: the same branch gives byte-identical text.
- A switch whose outgoing runtime is gone (no summary requested) still
  produces the header. No provider call is involved.
- The switch notice's "summary" means a model summary. When there is none, it
  says what Bridge carried: "carried forward: all 12 messages, 3 files and
  2 links (no model summary)".
- Git status is not read. The incoming model works in the same folder and can
  run it itself, and the handoff is built under the database lock, where a
  git subprocess must not run.

### F3 (D6). A chat reads its own history

- New tool `chats_history` on the `bridge` server, for top-level chats only,
  like `chats_search` and `chats_read`.
  - Args `{before?: integer, limit?: integer}`, unknown fields refused.
  - Returns the caller's own active-branch entries with `sequence < before`
    (default: all), newest page first, listed oldest first: user, assistant,
    worker and summary messages, and completed tool entries with their fact
    and their output.
  - Every string is scrubbed whole before it is clipped. Message text is
    clipped to 2000 chars and tool output to 1200.
  - `limit` defaults to 20, clamped to 1..=50. The reply has `nextBefore`
    (the oldest sequence returned, or null at the start) and the data note.
  - Amended during implementation: a page also stops at 24 KiB of entries, so
    the server's 32 KiB ceiling never cuts a page and `nextBefore` is always
    exact.
- Refused: a worker session or a session with a parent (404), and the route's
  own refusals (read-only access, rate limit) apply as for the other history
  tools.
- Registered everywhere the history tools are: core `TOOLS`, MCP tool
  definitions and server instructions, the tool-limit table (`Bounded`), the
  apps table, the Claude sidecar's read and top-level lists, and the OpenCode
  read-only deny list.
- The handoff names it: when the tail did not carry every conversation entry
  and the session is a top-level chat, the facts block adds
  `earlier: N older messages are not in this handoff; call chats_history with before=<S> to read them, tool output included.`
  `<S>` is the oldest sequence the tail carried.
  - Amended during implementation (found by the offline eval): when the tail
    carried every message but the branch has tool calls, whose output a
    handoff never carries, the block says so instead:
    `tool output: not in this handoff; call chats_history to read it.`

### F4 (D5). An account change on the same app carries the thread

- A same-harness switch between accounts with different continuation keys,
  where the adapter supports native resume and cross-account continuation
  (Claude, Codex; never OpenCode), a thread is stored, the chat is not an
  unstarted aside, and the thread's transcript exists in the old home, plans
  a carry: native continuation on the same thread id, no outgoing summary,
  no remembered-thread row.
  - Claude: `<old config>/projects/<folder>/<id>.jsonl` is copied to the same
    `<folder>` under the new config folder.
  - Codex: the rollout under the old shared home's `sessions/` (or
    `archived_sessions/`) is copied to the same relative path under the new
    shared home.
- The copy writes a temp file in the destination folder and renames it. It
  never writes outside the destination home. A thread id that is not
  `[A-Za-z0-9_-]+` is refused. A destination that is already newer than the
  source is kept as is. Source and destination resolving to the same file is
  a no-op.
- The copy runs before the commit, off the database lock. If it fails, the
  switch falls back to the ordinary handover (outgoing summary, remembered
  thread, projection) and records `session.thread_carry_failed`. A commit
  given a carry that was never copied also takes the handover.
- After a carry the head is `native` with the same thread id, and
  `sessions.provider_session_id` is unchanged. The notice says the
  conversation continues in the same session, carried to the new account.
- A removed old account, a missing transcript, OpenCode, or an adapter without
  the capability keeps today's handover. Same-home account changes keep
  today's native continuation.
- A failed resume of the carried thread uses the existing ladder
  (`session.resume_failed`, then `checkpoint_restored`).

## Unit Tests

`tool_facts` (new module):
- `a_codex_command_fact_unwraps_the_shell_and_keeps_the_exit_code`
- `a_claude_completed_entry_reads_its_command_from_the_started_entry`
- `an_opencode_edit_and_a_claude_write_name_their_paths`
- `started_entries_and_output_never_make_a_fact`
- `a_token_in_a_command_is_scrubbed`
- `a_run_of_tools_merges_into_one_bounded_line`

`restoration`:
- `tool_lines_ride_in_the_tail_and_messages_are_still_what_is_counted`
- `a_branch_without_tools_renders_as_before` (existing tests unchanged)
- `the_facts_header_exists_without_any_summary`
- `a_model_summary_comes_first_and_the_facts_follow`
- `links_and_files_are_newest_first_deduplicated_and_capped`
- `started_with_appears_only_when_the_tail_lost_the_first_ask`
- `the_handoff_names_chats_history_and_the_oldest_carried_sequence`
- `a_worker_handoff_never_names_chats_history`
- `the_facts_header_is_deterministic`

`provider_threads`:
- `the_delta_for_a_returned_thread_carries_tool_lines`

`history_tools`:
- `chats_history_pages_the_callers_own_branch_with_scrubbed_tool_output`
- `chats_history_refuses_workers_and_children`
- `chats_history_rejects_unknown_arguments_and_clamps_the_limit`

`thread_carry` (new module):
- `a_claude_transcript_is_copied_to_the_same_project_folder`
- `a_codex_rollout_keeps_its_relative_path`
- `a_newer_destination_is_kept`
- `a_thread_id_with_a_path_in_it_is_refused`
- `a_missing_transcript_is_not_carried`

`sessions`:
- `an_account_change_on_claude_carries_the_thread_and_resumes_natively`
- `a_carry_that_fails_falls_back_to_the_handover_and_is_recorded`
- `a_commit_with_an_uncopied_carry_takes_the_handover`
- `opencode_account_changes_never_carry`
- the existing notice tests updated for "no model summary".

Property tests (proptest):
- For any branch of messages and tool entries with arbitrary text (any
  Unicode), and any window: the handoff is at most the budget, never splits
  a char, is deterministic, keeps the header's first line, and every tool
  line is at most 480 bytes.
- For any JSON payload, tool-fact extraction never panics and its output is
  bounded.

## Integration / Functional Tests

- `live_turn` replay: A long Claude history with tool calls, switch to Codex
  with no live runtime, cold start: the compiled prompt's
  `restoration_context` section holds the facts header, a `tools:` line and
  the `earlier:` pointer. No `compaction` entry is written.
- MCP server: `tools/list` for a top-level chat lists `chats_history`; a
  worker's `tools/call chats_history` is refused.
- Sidecar `node --test sidecar/claude-agent/test/options.mjs` with the new tool
  in the allowed lists.

## Evals

Offline (in the suite, no model):
`handoff_eval::offline` seeds `testing/evals/model-switch-handoff/cases.json`.
These are labeled chats shaped like the live ones: long, with tool-heavy
turns, PR links, edits, an early planted fact, and planted credentials in
commands and output. For each case and window (Haiku-small, 200k, 1M):
- every labeled file and link appears in the facts block;
- the planted early fact is either in the handoff or returned by
  `chats_history` paging from the `before` the handoff names;
- no output (handoff or `chats_history` page) holds 12 characters of a planted
  credential;
- the handoff is within budget.

Live (ignored, needs real providers):
`handoff_eval::live`, in-process core on a scratch data dir with real Claude
and Codex. It seeds a chat longer than the incoming budget, switches with no
live runtime, then asks about the early fact and about the files and PR from
tool calls. It is scored on what the model answered and whether it called
`chats_history`.

## Mutation

`scripts/mutants-handoff.sh` runs cargo-mutants on the lines this branch
changed in `tool_facts`, `restoration`, `provider_threads`, `history_tools`,
`thread_carry` and the `sessions` carry path. Every surviving mutant is killed
with a test, or named as equivalent in the PR.

## Fuzzing

`src-tauri/bridge-core/fuzz/fuzz_targets/handoff.rs` (cargo-fuzz, nightly):
raw bytes become a branch of entries with arbitrary payloads and a window. The
handoff never panics, stays within budget, splits no char, and no planted
credential survives. A `chats_history` target fuzzes the arguments: only
documented statuses, never another chat's entry. Each target runs at least
5 minutes, with seeds committed.

## Smoke Tests

- `bun run build` green.
- `cargo test -p bridge-core` for the touched modules, then the full lib suite.
- `bunx vitest run` for any touched frontend test.

## E2E Tests (the real app)

Run against the branch-built `bridged` daemon on a scratch data dir with real
provider logins, driven through `bridge exec --method`:
1. Claude chat in a scratch folder: a turn that writes a file and prints a
   GitHub PR URL. Stop the runtime so no summary can run. Switch to Codex. The
   stored `prompt_compilations` row for the cold start has the facts header
   and the `tools:` line. Ask Codex which file was written and which PR was
   linked: it answers both.
2. Seed a long history (larger than Codex's handoff budget) with an early code
   word. Switch with no runtime and ask for the code word. Codex calls
   `chats_history` (seen in the forest) and answers it.
3. Account carry: a Claude chat on the user's own login is told a code word,
   then switched to the `personal` account. The transcript appears under
   `~/.claude_personal/projects/<folder>/`, the thread id is unchanged, and
   no `session.resume_failed` is written. Asked for the code word, it
   answers. Then switch back to the own login and check the same. Clean up
   the scratch project folders in both homes.

## Manual / cURL Tests

N/A. Covered by the E2E steps above through the `bridge` CLI.

## Out of scope

- OpenCode cross-account continuation.
- Harness-native compaction, budget constants, idle reaper limits.
- Git status in the facts block (see F2).
