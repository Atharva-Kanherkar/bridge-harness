# Claude discovery isolation — test contract

## Functional Behavior

- A Claude chat can load its plugin paths during a connector probe, even on a cold cache. Briefings and networked read-only workers continue to receive the connector configuration. A tools picker can read the last connector snapshot without starting or awaiting a probe.
- A cold cache waits only on the requested part. Concurrent readers of that part share one probe. At 60 seconds an expired entry is served immediately and one background probe starts. A failed refresh keeps the previous successful result and retries after the refresh interval.
- Both hosts warm plugins and connectors independently. A successful Bridge plugin mutation invalidates both parts, and connector authentication invalidates the connector part. The action waits for its refresh. A probe started before invalidation cannot restore the old configuration.
- Uncached discovery retains its CLI arguments, response parsing, diagnostics and timeouts. The existing chat and restricted-sidecar policy remains in force, with no wire change.

## Unit Tests

- Pause the connector probe with channels and verify the plugin read completes before the probe is released, including on a cold cache. Record elapsed time separately from the ordering assertion.
- Verify one cold probe serves concurrent readers. Advance an injected time source to verify expiry, stale reads during refresh, successful replacement and the failed refresh retry interval.
- Verify invalidation while an old probe is running rejects its result, and a caller after the action gets the new value. Verify the tools picker reads only the connector slot.

## Integration / Functional Tests

- Use a fixture Claude CLI to exercise plugin paths, connector health and server configuration. Compare cached discovery with the uncached discovery result for the same CLI state.
- Run the existing marketplace and Claude adapter tests to check sidecar launch scopes, connector discovery and CLI timeouts.

## Smoke Tests

- Run `bun run build` and `bun run test` before opening the PR.

## E2E Tests

- Use computer use on the desktop app with an isolated data directory. Start a new Claude chat while a deliberately delayed connector probe runs. Verify the message is delivered, the Claude response is visible and the local latency ledger records the launch.

## Manual Tests

- Compare `node scripts/latency-report.mjs --data-dir <path>` and `node scripts/latency-experiment.mjs chat --data-dir <path> --harness claude --n 3` on the same machine and separate daemon data directories. A single test session measures the isolated repro. The issue also calls for a day of live-install data, with the Claude `spawn` step at p95 of at most 150 ms.
