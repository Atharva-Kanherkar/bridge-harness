# fix/warm-resume-deadlock — Test Contract

Live incident, 2026-08-20: an orchestrator delegated research; the router chose
`resume_worker` for a compatible stopped worker from two days earlier. The
launch thread self-deadlocked and took the daemon down with it — a thread
sample showed `launch_worker_outcome::{{closure}}` parked in
`pthread_mutex_wait` on the store lock while every connection thread (forest
digest, snapshots, steering delivery) queued behind the same mutex. The worker
ran but never received its task; the orchestrator waited forever; the UI froze
on stale data.

## Root causes

1. **Chained-lock self-deadlock.** Both reuse arms chain lifecycle
   transitions in one expression:
   `transition(&state.db.lock()…, Restored, …).and_then(|_| transition(&state.db.lock()…, Working, …))`.
   Rust keeps the first statement's temporary `MutexGuard` alive until the end
   of the whole chained expression, so the second `lock()` re-locks the same
   non-reentrant mutex on the same thread. Present since the Arc refactor;
   only the rarely-routed reuse paths contain the chain, which is why fresh
   spawns never hung.
2. **Resume never re-parents.** `activate_reused_worker` resets the lease and
   `result_status`, but leaves `worker_runtime.parent_session_id`,
   `sessions.parent_session_id`, `sessions.depth`, and a stale
   `sessions.ended_at` pointing at the previous life. Even without the
   deadlock, the resumed worker is invisible under the new orchestrator and
   its result routes to a possibly dead parent.

## Functional Behavior

- Promoting a reused worker (hot `stopped→resuming→working`, cold
  `restored→working`) completes: each transition acquires and releases the
  store lock in its own statement.
- Activating a reused worker re-points `worker_runtime.parent_session_id` and
  `sessions.parent_session_id`/`depth` at the resuming orchestrator, clears
  `sessions.ended_at`, and resets `result_status` to `pending` (existing
  behavior preserved).

## Unit Tests

- `promote_restored_worker` and `promote_stopped_hot_worker` run to completion
  under a 10s watchdog (a reintroduced chained lock times the test out instead
  of hanging the suite) and land the runtime on `working` with lifecycle
  events recorded.
- `activate_reused_worker` re-parents both rows, clears `ended_at`, resets
  `result_status`, and re-encodes the compatibility key.

## Integration / Smoke

- `bun run build` and `bun run test` green.
- Live: rebuild the bundle, relaunch Bridge, confirm the daemon serves
  requests (digest polls answer) after a resume decision.

## E2E

N/A — the delegation resume path is exercised by the unit tests plus the live
relaunch; a full orchestrator turn needs a provider subscription session.

## Local refund checkpoint

Both promote helpers under watchdog, re-parent assertions, full frontend/Rust
suites, `git diff --check`.
