# feat/workflows-p1-trigger: Test Contract

Slice P1.2 of Workflows. The phase contract (`testing/feat-workflows-phase-1.md`) is the
authority; this file names what this branch must prove.

## Functional Behavior

Phase contract C3 to C5, C15, F8 to F15 and F13a, plus:

- `workflow_trigger::claim` is the only function that inserts a `workflow_runs` row. It runs
  in one immediate transaction: reconcile the project, refuse what may not run, return the
  existing run for a repeated occasion, record an overlap, or insert a `queued` run holding a
  15-minute lease.
- `heartbeat` extends a lease only for the holder of a live run; `finish` writes a terminal
  status only for the holder of an unexpired lease. Zero rows changed means the lease was lost,
  and the caller is told so.
- `reclaim_expired` settles every active run whose lease expired as `failed` /
  `lease_expired`, one compare-and-swap per row.
- `due_occasions(now)` returns at most one occasion per scheduled workflow, per C5 and F13a.
- `start_workflow_maintenance` ticks every 30 seconds on both hosts: reclaim, reconcile
  projects, claim due occasions, dispatch claimed runs. Until the runner lands (P1.3), dispatch
  settles a run as `failed` / `runner_unavailable`.
- New wire: `workflows/run_workflow`, `cancel_workflow_run`, `list_workflow_runs`,
  `read_workflow_settings`, `write_workflow_settings`, the `workflows/run_changed`
  notification (payload `{ run }`, serialized from the wire type), and `nextRunAt` / `lastRun`
  on `WorkflowSummary`.
- Cancelling a queued or waiting run settles it as `cancelled` and clears its lease; a running
  run's holder learns on its next heartbeat.
- Settings: `paused` and `quotaReservePercent` (default 25, range 0 to 90), stored under
  `configuration_entries(kind='workflows', id='settings')`; unpausing records the anchor.

## Unit Tests

Phase contract U-T1 to U-T11 (U-T4 lives in `workflow_schedule.rs`), plus:

- `heartbeat_and_finish_require_the_live_lease`
- `cancel_settles_a_queued_run_and_clears_its_lease`
- `settings_round_trip_and_refuse_out_of_range_reserve`
- `run_changed_payload_is_the_wire_run`

## Integration / Functional Tests

- Deck parity, drift and naming gates stay green.
- Daemon dispatch: `run_workflow` twice with one `requestId` returns one run, `created`
  true then false; `list_workflow_runs` returns it.
- I-4 host parity: both hosts start the maintenance loop.

## Smoke Tests

- A fresh data dir boots with the maintenance loop running and logs no tick error.

## E2E Tests

N/A for this slice; the runner lands in P1.3.

## Manual / CLI Tests

- `bridge exec --json --method workflows/run_workflow --params '{"workflowId":"<id>","requestId":"r1","dryRun":true}'`
  twice returns one run, `created: false` the second time, then `failed` with
  `runner_unavailable` until P1.3.
