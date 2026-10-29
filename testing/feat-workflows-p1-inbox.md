# feat/workflows-p1-inbox: Test Contract

Slice P1.6 of Workflows. The phase contract (`testing/feat-workflows-phase-1.md`) is the
authority; this file names what this branch must prove.

## Functional Behavior

Phase contract F34 to F38, plus:

- `workflows/mark_workflow_runs_read { runIds }` clears `unread` on those runs and publishes
  `workflows/run_changed` for each; unknown ids are ignored; the flag lives in SQLite, so it
  survives a restart.
- The Workflows page gains an **Inbox** section. "Needs you" lists runs waiting for approval,
  each with its pending card inline (Allow once, Always allow for this workflow, Decline) and
  runs waiting for quota. "Results" lists the rest newest first, unread first among runs of the
  same moment, each with its workflow, status, summary and when. Opening a run marks it read.
- The sidebar's Workflows row shows a count: runs waiting for the person plus unread results.
- Notifications (`notifyAttention`): a run that starts waiting for approval, waits for quota,
  or fails notifies once; a success notifies only when its workflow's `notify.onSuccess` is
  set. One notification per run and status, decided by a pure function.
- Mission Control shows a strip of active workflow runs, each opening its run view. (The dock's
  Agents pane shows a chat's own agents; a workflow run has no parent chat, so it is not there.)

## Unit Tests

- U-I1 `unread_state_survives_restart` (Rust)
- `mark_workflow_runs_read_ignores_unknown_ids_and_publishes_once_each` (Rust)
- V-5 the inbox orders runs, approves, declines, and clears unread on open
- V-7 Mission Control's strip lists active runs and opens one
- I-3 `workflowNotification` notifies each run and status once, and successes only when asked

## Integration / Functional Tests

- Deck parity and protocol gates with the new method; full vitest; build.

## Smoke Tests

- Mock mode: the inbox shows the waiting run with its card, the sidebar count, and Mission
  Control's strip, light and dark.

## E2E Tests

Covered by P1.7.

## Manual / CLI Tests

- In mock mode, approve and decline from the inbox; open a result and see the count drop.
