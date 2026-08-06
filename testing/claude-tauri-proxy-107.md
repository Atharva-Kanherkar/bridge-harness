# PR 132 adversarial fix contract

## Functional Behavior

- [x] More than four simultaneous desktop invokes complete with the response belonging to the request that issued them; a pooled client is never reused while its prior call is in flight.
- [x] A locally overflowed notification subscriber observes `stream-lagged` even when the daemon sends no later notification.
- [x] Desktop-managed daemons disable the process-global health listener so separate data directories can start concurrently.
- [x] `BRIDGE_DESKTOP_HOST` accepts only `auto`, `daemon`, and `embedded`; an unknown configured value fails startup instead of silently changing the acceptance host.
- [x] Source-checkout daemon discovery selects only the binary for `TAURI_ENV_TARGET_TRIPLE`.
- [x] A daemon child that never becomes reachable is killed and reaped before launcher failure or embedded fallback.

## Unit Tests

- [x] Cover single-flight call admission and timeout accounting in `bridge-client`.
- [x] Cover notification overflow followed by a quiet producer.
- [x] Table-test host preference parsing, including invalid input, without reading process-global environment.
- [x] Cover exact target-triple staged-binary selection.
- [x] Preserve registry uniqueness and wire-parameter parity checks.

## Integration Tests

- [x] Exercise more than `POOL_SIZE` concurrent proxy calls against a real daemon and verify all results.
- [x] Verify every desktop-spawned daemon command disables the global health listener.
- [x] Exercise startup failure cleanup with a child that remains alive without creating a socket.

## Smoke Tests

- [x] `bun run build` succeeds.
- [x] `bun run test` succeeds.
- [x] Targeted `bridge-client` and daemon-host suites succeed.

## E2E Tests

- [x] Existing Tauri command-registry parity and daemon notification forwarding tests remain green.

## Manual Verification

- [x] Review the final diff against fresh `origin/main` for unrelated changes, unbounded waits, child-process leaks, and fallback paths that can mask daemon-host failures.
- [x] Confirm fresh `origin/main` remains the merge base before push.
