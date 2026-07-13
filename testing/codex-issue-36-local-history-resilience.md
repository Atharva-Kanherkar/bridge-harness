# Issue 36 Contract: Durable local history without telemetry contention

- Correctness-critical session, queue, lease, and outbox transactions use only `bridge.db`; telemetry spans are batched into a separate `bridge-telemetry.db` connection after semantic commits.
- A telemetry failure never rolls back or relabels a committed semantic event.
- The controller periodically exports a transactionally consistent SQLite snapshot plus a SHA-256 manifest, and snapshot verification detects corruption.
- Product documentation describes the guarantee as durable local history, not a tamper-proof evidence ledger.
- Startup exposes and initializes the primary database, telemetry database, and snapshot directory independently.

## Local refund checkpoint

Verify database separation under locks/failures, telemetry batching, consistent export and checksum verification/corruption detection, periodic scheduling, documentation claims, full frontend/Rust suites, type checks, and `git diff --check` before the issue-only PR.
