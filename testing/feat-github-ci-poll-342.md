# feat/github-ci-poll-342 - Test Contract

## Functional Behavior

- The core watches only open pull requests whose check rollup contains queued or in-progress checks.
- While the application is focused, watched checks are refreshed every 15 seconds. While unfocused, the cadence is 120 seconds.
- Refocusing requests an immediate refresh rather than waiting for the next unfocused tick.
- A changed rollup invalidates the GitHub cache and emits a transient `github/checks_changed` event through the shared core event bus.
- A terminal rollup stops being watched and does not launch later `gh` check requests.
- GitHub rate-limit failures exponentially back off polling and create a health warning. Other polling failures do not crash the maintenance worker.
- Event subscribers that reconnect refetch authoritative data rather than relying on missed transient events.

## Unit Tests

- Poll scheduling selects focused, unfocused, and exponential rate-limit delays without real sleeps.
- A mutable fake `gh` fixture changes an in-progress check to failure, producing exactly one checks-changed event.
- Terminal check results are removed from the watch list and are not polled again.
- Rate-limit errors increase backoff and create a health warning.
- The protocol notification registry and `CoreEvent` map the checks-changed payload exactly.

## Integration / Functional Tests

- The daemon forwards `github/checks_changed` to subscribed clients using the existing event fan-out.
- The GitHub panel refetches its PR list and selected PR checks after a checks-changed notification.
- Window focus and blur report application focus state to the native boundary.

## Smoke Tests

- `bun run build` completes successfully.
- `bun run test` completes successfully.
- `bun run check` completes successfully.

## E2E Tests

N/A - deterministic Rust and jsdom tests cover the worker and its event-driven UI update; this repository has no browser E2E harness for the native GitHub surface.

## Manual / cURL Tests

- Run the application with an open PR whose GitHub Actions job is running, then confirm its badge changes without manual refresh when the job completes.
- Blur the application, confirm refreshes slow to the unfocused cadence, then refocus and confirm an immediate refresh.
- Simulate a GitHub API rate-limit response and confirm the health surface reports the warning without a tight retry loop.
