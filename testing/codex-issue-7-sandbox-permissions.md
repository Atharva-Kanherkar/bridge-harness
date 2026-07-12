# codex/issue-7-sandbox-permissions — Test Contract

## Functional Behavior

- Worker `writeMode` is passed as typed launch configuration through the adapter registry; prompt text is never the enforcement mechanism.
- Codex launch mapping is deterministic: read-only and scoped writers use `workspace-write`; only `full` workers use `danger-full-access`. Approval policy is mapped explicitly per mode.
- Claude launch mapping is deterministic: read-only workers use non-bypass permissions and explicit write-tool denials; scoped writers use non-bypass edit permissions; only `full` workers use `bypassPermissions`.
- The depth-zero orchestrator remains a separate trusted launch path and cannot be confused with a worker lease.
- Before a read-only worker starts, the supervisor captures tracked-file `git status --porcelain --untracked-files=no` for its workspace.
- When a read-only worker completes or exits, the supervisor compares tracked status and appends `worker.read_only_violation` if it changed. Untracked build artifacts do not trigger violations.
- Violation events use the child session ID so the existing workspace event feed displays them.

## Unit Tests

- Codex thread-start parameter tests cover read-only, shared, isolated, full, and orchestrator mappings; no non-full worker receives danger-full-access.
- Claude argument tests cover the same modes; no non-full worker receives bypassPermissions and read-only denies write tools.
- Tracked-status helper excludes untracked files and reports tracked modifications.
- Verification helper records one violation event when baseline/current tracked status differ and none when unchanged.

## Integration / Functional Tests

- Runtime worker launch forwards the lease write mode into the chosen adapter before process start.
- Read-only baseline is registered only after a successful reservation and before adapter start, and is cleared after completion/exit verification.
- Existing policy reservation, typed delegation, tier resolution, and result relay behavior remain green.
- Ignored live Codex test attempts a tracked-file edit in read-only mode and verifies no tracked modification; ignored live test worker can run `cargo test` while producing only untracked artifacts.

## Smoke Tests

- `bun run test` passes.
- `bun run check` passes.
- `bun run build` passes.
- `git diff --check origin/main...HEAD` passes.

## Acceptance Audit

- Static audit proves the only worker path to `danger-full-access` / `bypassPermissions` is `WriteMode::Full`.
- Querying `events` after a simulated tracked change returns `worker.read_only_violation` for the child session.
