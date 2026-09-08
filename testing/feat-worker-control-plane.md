# Worker control plane contract (issue #572, slices 3–5)

## Failover on provider exhaustion (slice 3)
- A worker that dies of provider exhaustion has its objective relaunched once, on whatever harness routing still finds eligible. Bridge does not pick the harness itself: it clears the harness and model hints and lets the router's existing hard-supply-gap substitution answer.
- The relaunch spends the objective's one automatic attempt. An objective with none left is not rerouted.
- Either outcome reaches the orchestrator and the user: a `bridge-worker-rerouted` notice naming the exhausted harness, its reset time and the substitute, or a blocked notice naming the reset time when nothing else can serve it.

## Stop verb (slice 4)
- `bridge-stop` (`{"sessionId","reason"}`) is a typed block alongside delegate, peek and steer. It targets only the orchestrator's own workers, requires a reason, and refuses a worker that has already finished.
- A stop interrupts the worker, settles it `cancelled`, tears down the process, and reports to the parent — through the same path the user's stop takes.
- Cancellation settles from any lifecycle state, including `waiting`. A worker that had already reported still produces a visible cancellation on the parent rather than being swallowed by the claimed result seam.
- Stopping is reachable from the worker card and the tasks pane in every non-terminal state.

## Steer and peek (slice 5)
- Pending peeks, steers and stops are queues, not single slots: every request in a turn is delivered, in order, with stops last.
- They drain when the turn ends *or* errors, so a failed turn does not strand them.
- A refusal is a visible `delegation.rejected` entry as well as a ledger row, so it survives an orchestrator whose adapter is gone.
- A steer is never silently queued because Bridge could not read the session's turn state; it is refused with that reason.
- A peek that finds nothing says which nothing: already reported, not yet active, not your worker, or no live workers at all.

Verification: parse tests for the stop block including empty reasons, foreign session ids and duplicate blocks; behavioural tests for stop-cancels-and-reports, stop-aimed-elsewhere-is-refused-visibly, waiting-worker-cancellation, cancellation-after-reporting, multi-steer delivery, the three peek miss reasons, failover budget exhaustion, and the no-eligible-harness announcement. Run `bun run build` and `bun run test` before opening the PR.
