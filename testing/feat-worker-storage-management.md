# Worker and Storage Management Test Contract

Base audited: fresh origin/main at 4473707.

## Functional Behavior
- Preserve the shipped durable repair budget, provider-limit failover, stop protocol,
  queued steer/peek delivery and typed-result safeguards. Do not reimplement them.
- Mission Control and worker detail expose stop, harness/model, terminal elapsed
  time and operational diagnostics. No runtime is not evidence of success.
- Workers settings expose real persisted controls, with validation and honest
  descriptions of scope; routing configuration is reachable from Settings.
- Storage provides search/filter/sort, per-repository usage, measurement freshness,
  explicit reclaim confirmation and visible errors. External worktrees remain
  reported only. Dirty, live, unadopted and unverifiable checkouts remain protected.
- Archived chats are searchable in Settings and their content can be read.
  Unarchive changes visibility only: no adapter starts, checkout is restored, or
  unrelated conversation changes. Repeated unarchive is harmless.
- Worktree observations have deadlines and report incomplete observations without
  using them as proof of safe deletion. Worker creation does not hold the DB mutex
  across git/filesystem work.
- Dependency seeding must not expose a dependency tree from a different lockfile;
  unavailable copy-on-write falls back to the normal dependency-install workflow.

## Unit Tests
- Worker status/terminal clocks and diagnostic projection, including no runtime,
  cancelled, quota, reroute, retry refusal and unreadable result.
- Archive list/unarchive persistence, missing id, descendant visibility and no
  filesystem/provider side effects.
- Storage search/filter/sort, confirmation, refusal and failed request behavior.
- Settings validation/persistence and actual runtime consumption.
- Dependency seed identity checks, clone independence and fallback.
- Bounded repository commands, slow-command timeout and incomplete size reporting.

## Integration / Functional Tests
- Rust protocol registry/schema drift checks and both dispatch paths stay aligned.
- Run affected Rust and Vitest suites during each increment.
- Run bun run build and bun run test before opening the PR.

## Smoke Tests
- Browser preview: Settings Storage, Archived chats and Workers; Mission Control
  with live, waiting, failed and completed fixtures, including a narrow viewport.
- Verify error/loading/empty states and keyboard-accessible actions.

## E2E Tests
- Archive a stopped chat, find/read it in Settings, unarchive, then find it in the
  sidebar without starting it. Rust round-trip plus UI integration coverage.
- Reclaim flow is tested against temporary git repositories only, not user data.

## Manual Tests and Limits
- Signed-release disk measurements and provider-account failover require a real
  release/live account; report unexecuted checks rather than inferred success.
- APFS clone allocation cannot be established by summing logical du sizes alone.
- Do not remove the issue's orphan checkout or existing database backups as part
  of development. Do not change billing settings to unblock GitHub Actions.
