# Orchestration reliability: unlimited workers, waits that see the queue, recovery

Found by delegating four T3 Code research workers from one orchestrator turn
(Claude, Codex, OpenCode, and one pinned to a model that does not exist), then
reading the live database and source.

## Contract

### Worker counts are not capped by default
- `PolicyEngine::default()` spawns a read-only worker with 40 active and 40
  already spent this turn, and an isolated writer with 40 active writers on
  other paths.
- `WorkerSettings` defaults `maxConcurrentWorkers` and `maxWorkersPerTurn` to
  `0`, meaning no limit; `worker_settings::policy` maps `0` to `usize::MAX`.
  Saving accepts 0-64.
- The forest snapshot reports an unlimited limit as `0`, never `-1`.
- One writer per worktree, write scope and its provenance, depth, approvals and
  independent verification are unchanged.

### `bridge-wait` covers queued workers
- `"all"` includes the orchestrator's queued delegations as `queue:<id>`.
- `all_done` does not fire while a queued delegation is still queued, or while
  it is dispatched and its session has not reported.
- A dispatched queue entry is swapped for its session (`adopt_dispatched`), so
  its result is held with the rest.
- A rejected, expired or cancelled queue entry no longer holds the wait.
- `stillRunning` names queued entries.
- A listed wait accepts `queue:<id>` or the bare queue id.

### The fleet digest lists queued workers
- Entries have `lifecycle: "queued"` and `sessionId: "queue:<id>"`.

### Queue dispatch
- One completion dispatches every queued worker that can start, not one.
- A dispatched launch that fails is closed with a `last_error` and the
  terminal receipt, so the orchestrator is told once, with the reason, at
  launch, and not again as a bare "worker launch failed".

### Approvals the agent already settled
- A provider 404 / `PermissionNotFoundError` when answering an approval records
  a resolved interaction, not a failed one, and returns the session to
  `working`. Before this, an OpenCode worker sat in `waiting /
  permission_requested` while running tools.

### Recovery
- A real provider usage limit (worker already `failed` from the error frame)
  reaches the reroute. Before this, the early return in
  `settle_worker_after_result` skipped it for every real limit.
- When no other harness can take it, the objective is scheduled to relaunch at
  the harness's reset plus 90 s, at most 3 times per objective, and the
  orchestrator gets `bridge-worker-resume-scheduled`.
- A due resume relaunches once, even across maintenance ticks.
- A worker whose process exited (Bridge's synthetic exit result) or failed
  transiently is relaunched once per objective, gated by Automatic retry. A
  stall, a quota wall or a formatting failure is not relaunched.
- Every relaunch (manual retry, reroute, resume, crash) carries what the failed
  attempt did as a known fact, built from its forest branch.

### Ending an orchestrator ends its work
- `stop_session` on an orchestrator cancels its queued delegations and stops its
  live workers. Before this, only archive cascaded.

## Not covered
- Interrupting an orchestrator's turn (as opposed to ending the session) still
  leaves its workers running, by design: an orchestrator is often interrupted to
  redirect it while its workers continue.
- Native resume of the failed provider session is not attempted; the handoff is
  Bridge's forest summary.
