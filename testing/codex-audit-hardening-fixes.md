# codex-audit-hardening-fixes — Test Contract

## Functional Behavior

- Starting a session in a workspace with a `NULL` path uses the same private scratch directory as a folder-less chat; it never fails while decoding the database value.
- Provider stdin lock poisoning returns a typed adapter error to the caller. It must not panic the supervisor thread, and a send failure leaves the session recoverably failed.
- Worker queue claims expire old queued work, reclaim stale `dispatching` claims, reject malformed/poisoned requests, and pick the oldest eligible work fairly across workspaces.
- State transitions that request an external delivery enqueue a durable, idempotent outbox record in the same SQLite transaction. Delivery attempts use bounded exponential backoff and terminate in a dead-letter state.
- Trace identity flows through turns, delegations, queue work, adapter requests, tools, and usage records; OpenTelemetry-compatible GenAI spans/metrics are emitted for model/provider/token/latency/finish reason/delegation/tool lifecycle.
- A versioned `handoff_packet` defines portable Codex ↔ Claude continuation data, validates required fields, and is persisted as evidence before handoff.
- The change adds only the above durable primitives and does not introduce a second read model or workflow engine.

## Unit Tests

- `start_session` path resolution returns the private scratch path when the workspace path is absent.
- Codex and Claude writer helpers return `BridgeError::Adapter` when their mutex is poisoned.
- Queue maintenance expires aged work, reclaims stale dispatches, and selects fairly between workspaces.
- Outbox enqueue/dequeue uses a stable idempotency key, applies backoff, and dead-letters after the retry limit.
- `HandoffPacket` serializes with schema version 1 and rejects missing/unsupported data.
- Trace context attaches to persistence records and produces valid OpenTelemetry field values.

## Integration / Functional Tests

- SQLite migration creates queue lifecycle, outbox/inbox, evidence, handoff, and trace columns/tables without altering existing saved data.
- A failed provider turn records a recoverable failed session rather than unwinding the supervisor.
- Parent turn → policy decision → queued worker → adapter request retains one trace ID.

## Smoke Tests

- `cargo test` passes in `src-tauri`.
- `cargo clippy --all-targets -- -D warnings` passes in `src-tauri`.

## E2E Tests

N/A — provider binaries and external integrations are not available in the unit-test environment.

## Manual / cURL Tests

1. Create a workspace without a folder, start its session, and verify the session starts using the app-private scratch directory.
2. Inspect the SQLite database after an integration-notification state transition: one outbox record with an idempotency key exists; repeated delivery is idempotent.
