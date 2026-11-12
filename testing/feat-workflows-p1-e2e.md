# feat/workflows-p1-e2e: Test Contract

Slice P1.7 of Workflows, the last of Phase 1. The phase contract
(`testing/feat-workflows-phase-1.md`) is the authority; this file names what this branch must
prove: that the whole journey works through the daemon, not just each slice on its own.

## How the journeys run

- A real daemon (`bridged`) on a temporary data directory, driven over its Unix socket by a
  JSON-RPC client. E3 also runs the real `bridged` binary, so it can be killed for real.
- Only the model is fake. `BRIDGE_CLAUDE_SIDECAR` points at
  `testing/fixtures/workflow-e2e/sidecar.mjs`, which runs Bridge's real Claude sidecar
  (workflow gate, ask relay, control frames) against `fake-sdk.mjs`, a stand-in for the Claude
  Agent SDK whose `query()` scripts its replies from markers in the run's instructions. Every
  write that actually executes is appended to `writes.log` in the run's folder.
- A fake `claude` CLI first on `PATH` (`testing/fixtures/workflow-e2e/bin/claude`) reports two
  connected connectors, `claude.ai Slack` and `claude.ai Linear`, so discovery is hermetic.
- The fake clock is `workflow_trigger::tick(core, now)` called with a chosen instant; the
  daemon's own loop keeps running on real time and must not interfere.
- Draft replies are the recorded Haiku replies from `testing/fixtures/workflow-drafts/cases.json`.

## Functional Behavior

- E1 No-project journey (F1, F13, F16 to F22, F32, F34 to F36):
  - `draft_workflow_from_text` with case 3's sentence returns a Run-at-08:30 schedule, Linear
    read and Slack write-asks grants, and saves nothing.
  - Saved, it needs approval and has no project. A dry run before approval succeeds without
    executing a write and records a `previewed` effect.
  - Approved, the schedule fires at the fake clock's next slot and creates one `schedule` run.
  - The Slack write waits for the person; `approvals/resolve_approval` with accept lets it run.
  - The run succeeds with the model's summary, exactly one write executed, one `approved`
    effect, and the run is unread in the inbox listing.
  - The run's conversation is openable (`get_session_forest`) and forkable (`fork_session`) into
    a visible chat; marking the run read clears it from the unread listing.
- E2 Project journey (F16, C5): on a real temporary Git repository added as a project, a
  `files: "worktree"` run edits a file in its own worktree and succeeds; the original
  checkout's `git status --porcelain` is unchanged and the file is not in it.
- E3 Restart mid-run (F11, F22): a real `bridged` process runs a workflow that executes one
  approved write and then waits on a second. The process is killed with SIGKILL. A new daemon
  on the same directory reclaims the run once the lease has expired on the fake clock: `failed`
  with `lease_expired`. `writes.log` still has exactly one write, and the effects table shows
  one `approved` write.

## Unit Tests

None new; the journeys reuse the slices' units.

## Integration / Functional Tests

- `src-tauri/bridged/tests/workflows.rs`: E1, E2, E3 as above, each on its own data directory.
- They need Node 18+ on `PATH`, as the sidecar does; without it they fail with that reason
  rather than skip.

## Smoke Tests

- `bun run test` runs them as part of the workspace's cargo tests.

## E2E Tests

E1 to E3 above.

## Manual / CLI Tests

A release build of the app, not `tauri dev`, on a fresh data directory, light and dark:

1. Workflows in the sidebar; the empty page; describe a job; the drafted editor.
2. Save, dry run, approve; the detail page with its next run.
3. A run waiting for a write: the inbox card, the sidebar count, the Mission Control strip.
4. Allow once; the finished run; Continue in chat.

Screenshots are attached to the PR. Where a step needs a real connector or a real schedule
slot, the PR says so instead of faking it.
