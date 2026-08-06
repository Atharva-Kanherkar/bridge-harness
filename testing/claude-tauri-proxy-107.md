# PR 132 adversarial fix contract

## Functional Behavior

- [ ] More than four simultaneous desktop invokes complete with the response belonging to the request that issued them; a pooled client is never reused while its prior call is in flight.
- [ ] A locally overflowed notification subscriber observes `stream-lagged` even when the daemon sends no later notification.
- [ ] Desktop-managed daemons disable the process-global health listener so separate data directories can start concurrently.
- [ ] `BRIDGE_DESKTOP_HOST` accepts only `auto`, `daemon`, and `embedded`; an unknown configured value fails startup instead of silently changing the acceptance host.
- [ ] Source-checkout daemon discovery selects only the binary for `TAURI_ENV_TARGET_TRIPLE`.
- [ ] A daemon child that never becomes reachable is killed and reaped before launcher failure or embedded fallback.

## Unit Tests

- [ ] Cover single-flight call admission and timeout accounting in `bridge-client`.
- [ ] Cover notification overflow followed by a quiet producer.
- [ ] Table-test host preference parsing, including invalid input, without reading process-global environment.
- [ ] Cover exact target-triple staged-binary selection.
- [ ] Preserve registry uniqueness and wire-parameter parity checks.

## Integration Tests

- [ ] Exercise more than `POOL_SIZE` concurrent proxy calls against a real daemon and verify all results.
- [ ] Start desktop-managed daemon processes for two isolated data directories without a health-port collision.
- [ ] Exercise startup failure cleanup with a child that remains alive without creating a socket.

## Smoke Tests

- [ ] `bun run build` succeeds.
- [ ] `bun run test` succeeds.
- [ ] Targeted `bridge-client` and daemon-host suites succeed.

## E2E Tests

- [ ] Existing Tauri command-registry parity and daemon notification forwarding tests remain green.

## Manual Verification

- [ ] Review the final diff against fresh `origin/main` for unrelated changes, unbounded waits, child-process leaks, and fallback paths that can mask daemon-host failures.
- [ ] Confirm PR #132 still points at the pushed head and is mergeable.
