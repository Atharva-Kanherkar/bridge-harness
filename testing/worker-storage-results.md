# Worker and storage verification

## Connector review follow-up

The four connector findings are covered by
[worker-storage-review-fixes.md](worker-storage-review-fixes.md).

- Defaults and explicit overrides respect harness/model exclusions in Shadow and
  Disabled modes; an eligible substitute is selected or launch is refused.
- Archived transcripts use the shared renderer, including non-message events and
  tool pairs spanning fetched pages. Historical approval, permission, question and
  prompt-change controls are disabled. Metadata-only pages explain their state.
- Archived roots expose and are searchable through their recursive descendants.
  Normal state hides the entire archived subtree; independent child archives
  survive root unarchive. Reading descendants restores no checkout or provider.
- A real Git alias producing 17 MiB verifies caller-specific prefix truncation;
  whole-output reads still reject overflow and deadline coverage remains green.

Follow-up verification: `bun run build` and `bun run test` passed. Vitest has
2,065 passing tests across 162 files; the Rust workspace/doctests have 2,583
passing tests with the same 13 explicit ignores. Protocol generation and the
Tauri command-surface checks passed. The environment isolation below was reused.

## Initial implementation

Implementation tested at `76c1cbf`, including main through `93b57f4`.
Contract: [feat-worker-storage-management.md](feat-worker-storage-management.md).
Audit: [worker-storage-audit.md](../docs/worker-storage-audit.md).

## Final commands

- `bun run build`: passed. Existing Vite chunk-size/dynamic-import warnings remain.
- `bun run test`: passed end to end, including release tests, sidecar, Vitest and
  the complete Cargo workspace and doctests.
- Vitest: 162 files, 2,061 tests passed.
- Rust workspace: 2,580 passed; 13 explicitly ignored live/external checks.
- Release Node tests: 16 passed; release Python tests: 12 passed.
- Claude sidecar: 50 passed, one existing skipped check.
- `git diff --check`: passed.

Tests used `RUST_TEST_THREADS=8`. Bridge's inherited `CARGO_TARGET_DIR`,
`BUN_INSTALL_CACHE_DIR` and `npm_config_cache` were unset for the test process:
the existing build-cache integration test expects it can apply at least one
variable. A separate `CARGO_BUILD_TARGET_DIR` prevented other worktrees' builds
from replacing this branch's protocol artifacts during validation.

## Regression coverage

- Archived chat listing and unarchive round-trip, repeated unarchive, missing id,
  unchanged checkout absence, unchanged ended state, no provider or turn started.
- Archive UI read-only content, pagination affordance, unarchive-only API wiring,
  and failure that leaves the conversation visible without a false success flash.
- Worker settings validation/persistence and policy consumption; disabled retry
  and failover; longer stall deadline; zero warm retention closes the worker.
- Mission Control missing-runtime honesty without resurfacing ended historical
  workers; neutral cancellation; terminal clocks and bounded diagnostics.
- Reclaim confirmation and refusal behavior; external and protected checkouts
  remain non-actionable. Existing deletion-time safety tests stay green.
- A blocked Git post-checkout hook proves the DB is available while the worker's
  intended canonical path is already reserved against maintenance.
- Slow child process and inherited-pipe timeout; incomplete size measurement.
- Changed lockfile never seeds dependencies; a successful clone has distinct
  inodes and independent writes. Unsupported cloning leaves no partial install.
- Protocol naming, payload, generated-artifact and Tauri command-surface checks.

## Browser smoke

Local headless Chromium against the actual Vite app in preview mode:

- Storage at 1280x900 and 390x844; no horizontal page overflow.
- Reclaim opens confirmation; Cancel leaves the checkout listed.
- Workers page loads its controls and workspace selector.
- Archived transcript reads preview messages; Unarchive returns to the archive
  list with the explicit checkout-not-restored feedback.
- No browser page errors in the smoke run.

The archive flow uses isolated preview fixtures, not the user's archived data.
Native persistence and filesystem behavior are covered by the Rust regressions.

## Not claimed

No signed-release live-agent disk benchmark, forced provider-quota experiment,
or cleanup of the user's unverifiable directory was performed. Logical directory
sizes are not unique physical allocation on APFS. No account billing settings
were changed, and local passing tests do not imply GitHub Actions started.
