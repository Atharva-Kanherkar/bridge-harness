# Bridge 0.5.3 macOS validation

Validated on the same Apple Silicon Mac used for the downloaded 0.5.2
installation test, on September 8, 2026. Runtime fix: `696c2b0`; release
version metadata: `4bc5a23`.

## Fix and automated checks

The public 0.5.2 installation reproduced a clean-shutdown defect: an idle
provider's durable process claim survived normal Quit, so startup recovery
marked its successful chat failed. The fix settles process and turn ownership
atomically during daemon shutdown, preserves provider resume IDs and terminal
outcomes, covers a reader that already removed its runtime, and prevents a
late reader frame from undoing shutdown settlement.

- Production frontend build passed.
- Full release test gate passed: 127 frontend suites / 1,644 tests; 2,096 Rust
  tests with 13 existing ignored cases; sidecar and release-script checks.
- New regressions exercise real tracked fixture processes through daemon
  shutdown/restart, preserved answers and resume IDs, genuine terminal
  outcomes, and a late provider frame.
- Both app and DMG passed Developer ID signing, notarization, stapling,
  Gatekeeper, icon, entitlement, version, and bundled-helper verification.

## Installed application regression

The notarized app was installed at `/Applications/Bridge.app` with the user's
existing default application data. The previous installation and a data
backup were retained outside the repository.

A live Codex response completed with `READY-053`. Before normal Quit, the
session was `ready`, with no active turn and a tracked provider process. After
Quit, before reopening, the database already held `stopped`, with both process
ownership fields and the active turn cleared. The provider resume ID remained
unchanged. Both desktop and daemon exited.

Reopening preserved `stopped`, the answer, and the resume ID. No new
`adapter.request_failed` event followed the corrected `app_shutdown` event.
The earlier failure event at the first 0.5.3 launch belongs to the stale claim
left by the previous 0.5.2 installation; historical failure events are retained.

These are bounded installation and lifecycle checks, not a claim that every
provider or long-running workload has been validated.

## Artifact

`Bridge_0.5.3_aarch64.dmg` — 144,117,658 bytes.

SHA-256:
`71d93b8d8a6c18652f433405c572e41d9f876f01bd9fc5da117c4e2764d0eb09`.
