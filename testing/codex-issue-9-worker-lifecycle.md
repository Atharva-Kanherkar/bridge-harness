# codex/issue-9-worker-lifecycle — Test Contract

## Functional Behavior

- Worker lifecycle is persisted and restricted to the issue #1 state graph: starting → working; working ↔ waiting; working → warm/checkpointing/failed/completed/cancelled; warm → working/checkpointing; checkpointing → stopped; stopped → resuming; resuming → working/restored; restored → working; failed → resuming/completed; waiting → cancelled.
- Illegal transitions fail without changing the session row or appending a misleading forest event.
- Every successful transition updates durable state and appends `session.status` before any UI emission.
- Worker bookkeeping is derived from SQLite after restart; in-memory caches are optional and never authoritative. Working/waiting/warm children cannot leave a parent waiting forever.
- Active leases for dead working/waiting/warm sessions expire during app-start recovery before queued work is considered.
- Compatibility uses workspace + role + harness + capability tier + task family + normalized owned paths; model identity alone never permits reuse.
- Fast/read-only and strong workers terminate after a typed result; reusable standard implementation workers become warm for five minutes by default.
- Warm timeout requests checkpointing then stops the worker; a compatible future task selects hot/native/checkpoint restoration through the existing adapter restoration path.
- Queued launches are durable and dispatch FIFO when the conflicting lease is released, while still passing the deterministic policy gate.
- User cancellation interrupts an active turn, persists `cancelled` with reason `user_cancelled`, releases/expires the lease, reports one typed terminal `cancelled` result to the parent, and is never retried.
- Parent notification accepts typed worker results only and durable result/report markers prevent duplicate delivery after restart.
- `lib.rs` retains Tauri command plumbing while supervisor, worker pool, policy wiring, worktree coordination, and the #10 compaction-controller boundary live in focused modules.

## Unit Tests

- Exhaustive transition-table tests cover every legal edge and representative/all illegal pairs.
- Transition persistence is atomic with the matching `session.status` forest entry.
- Compatibility-key equality includes all six required dimensions and normalizes owned paths deterministically.
- Warm-retention policy covers fast/read-only immediate stop, standard implementation five-minute warm, and strong immediate stop.
- FIFO queue selection is stable by creation sequence/time and skips entries whose lease conflict remains active.
- Cancellation result is typed, terminal, non-retryable, and idempotently reportable once.

## Integration / Functional Tests

- Restart fixture with working, waiting, and warm workers restores/settles durable bookkeeping, expires dead leases, and unblocks every affected parent.
- Completing/cancelling a worker decrements durable outstanding state exactly once and releases its lease in the same transaction.
- Warm timeout persists checkpointing before stopped; compatible work selects that session, incompatible work does not.
- Releasing a writer lease dispatches the oldest eligible queued request.
- Supervisor and pool APIs operate using one caller-owned SQLite connection, preserving the repository's single-writer discipline.

## Smoke Tests

- `bun run test` passes.
- `bun run check` passes.
- `bun run build` passes.
- `git diff --check origin/main...HEAD` passes.

## E2E Tests

- N/A — provider-authenticated cancellation/resume is covered by opt-in adapter live tests plus deterministic supervisor/DB integration tests; no desktop E2E driver exists in this repository.

## Manual / Acceptance Audit

- Kill/reopen fixture audit proves no parent remains waiting solely because a child process disappeared.
- Static audit confirms no in-memory spawned/outstanding/reported map is authoritative.
- Static audit confirms transition persistence precedes UI emission and cancellation has no retry path.
- Module audit confirms `lib.rs` is command/orchestration plumbing rather than the owner of lifecycle and queue rules.
