# fix/stream-latency-and-tool-phase — Test Contract

## Observed defect

A Claude chat with ~1,650 forest entries showed a Bash card as "Running Bash" for
seven minutes while the model was still writing a `gh pr create` body; the PR
appeared on GitHub before the card changed. Tool durations on cards (2m 14s for a
3 s commit) counted the model's argument streaming as run time. Text streamed in
bursts. `bridged` sat at ~88 % CPU: every provider frame — including every text
delta of every live session — ran `CompactionController::pending`, which loaded
and JSON-parsed the whole active branch under the global database mutex, and the
one-second worker-maintenance tick ran `git status` in stopped worktrees while
holding the same mutex.

## Functional Behavior

- `CompactionController::pending` answers from one indexed existence probe when
  a session has never carried a compaction marker, and otherwise walks parent
  pointers from the active head to the nearest marker. It never loads the branch.
  Its answer is identical to scanning the loaded active branch: a request on an
  abandoned branch is not pending even when it is the session's newest marker.
- The live-frame handler asks for a pending compaction once per frame; the
  per-event lookup runs only while a checkpoint turn is active, when it can
  differ from `None`.
- `session_entries` gains `idx_session_entries_session_kind` idempotently on open,
  without a schema-version bump (and so without a full-store migration backup).
- Terminal worker worktrees are collected every 30 s, not every second; Git runs
  under the database lock at most that often.
- A Claude `tool_use` block streamed via `content_block_start` is a
  `*.started` event with `data.phase = "preparing"`; the assistant snapshot
  re-emits the same item with `data.phase = "running"` and restarts the tool's
  clock. `durationMs` on the completed card measures execution only.
- The transcript labels a preparing call "Preparing <ToolName>" and switches to
  the tool's normal verb ("Running <command>") when the snapshot lands.

## Unit Tests

- `compaction_controller::tests::pending_follows_the_active_branch_not_the_newest_entry`
- every existing `compaction_controller::tests::*` case that asserts `pending`
  before and after `begin`, `handle_output`, repair and background landings —
  unchanged, now exercising the SQL path.
- `agent::tests::claude_tool_phase_and_duration_start_at_the_snapshot`
- `agent::tests::claude_stream_start_and_snapshot_keep_one_tool_item` (unchanged
  item identity across the two phases).
- `src/transcript/toolCall.test.ts` — "says a Claude tool is being prepared until
  its arguments have landed".

## Verification on live data

- Against the 2.7 GB daily-use store: existence probe 1.7 ms unindexed on a
  1,651-entry session (sub-millisecond indexed); marker walk 1.2 ms on a
  4,629-entry compacted session; worst-case walk to root 12.7 ms, versus a full
  branch load plus JSON parse of every payload per frame before.
- `bun run test` green (sidecar, vitest, cargo workspace).
