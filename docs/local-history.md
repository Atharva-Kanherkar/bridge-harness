# Durable local history

Bridge keeps correctness-critical state—session entries and heads, worker leases and queues, approvals, usage, and the outbox—in `bridge.db`. This is durable local history, not a tamper-proof or replicated evidence ledger.

Provider telemetry uses a separate `bridge-telemetry.db`. Normalized spans are batched only after their semantic transactions commit, so a telemetry writer lock or failure cannot delay, roll back, or relabel correctness-critical state.

At startup and every 15 minutes, Bridge exports a transactionally consistent SQLite snapshot under `history-snapshots/`. Each export has a versioned manifest containing its filename, creation time, and SHA-256 checksum. Verification recomputes the checksum so disk corruption or accidental modification is visible. These local snapshots reduce single-file loss risk; copying the snapshot directory to another device remains the user's backup boundary.
