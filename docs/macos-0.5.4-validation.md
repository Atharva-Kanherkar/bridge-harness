# Bridge 0.5.4 macOS validation

Validated on the user's Apple Silicon Mac on September 8, 2026. The release
integrates `main` through `65b00ad90ff9efccc7092700d5d7053e2c7ee885` and
retains the 0.5.2/0.5.3 macOS fixes. Build source: `06d05c6`.

## Integration and automated checks

- Preserved native startup error handling, window lifetime fixes, desktop and
  daemon ownership, icon generation, and strict public release gates.
- Combined clean shutdown settlement with main's detached model-switch summary
  cancellation, retaining process identity checks and late-frame protection.
- Preserved main's streaming timing and pinned orchestrator model defaults.
- Isolated browser storage in the keyboard-navigation test: Node 25's incomplete
  global storage had shadowed jsdom's storage. Keyboard/focus assertions remain,
  with an added theme persistence assertion; no production workaround was added.
- Production frontend build and full release test gate passed: 140 frontend
  suites / 1,848 tests; 2,270 Rust tests / 13 existing ignored cases; sidecar and
  release-script tests passed.

## Installed application

The signed candidate was installed at `/Applications/Bridge.app` after backing
up the previous app and default application data. Apple accepted the app, and
the installed copy passed stapling, Gatekeeper, signature, entitlement, icon,
version, and bundled-helper checks.

The new interface visibly rendered. A live Claude Haiku turn read only the
first three README lines and correctly returned the `Bridge` heading. Native
fullscreen entry and exit during the turn rendered correctly. Normal Quit
exited the desktop and daemon, changed the chat from `ready` to `stopped`,
cleared its process and active-turn claims, and retained its provider resume ID.
Reopening displayed the saved answer; a follow-up using only the conversation
again answered correctly with the same provider resume ID. No new Bridge or
WebKit crash report appeared during these bounded checks.

Codex was not exercised in this Finder-launched installation: its executable
exists inside the Codex app's Resources directory, which is in the coding
task's PATH but outside the installed Bridge daemon's PATH and fallback
directories. Runtime discovery is unchanged from 0.5.3. The successful live
provider test here covers Claude, not every optional harness or long workload.

## Public artifact

`Bridge_0.5.4_aarch64.dmg` — 146,314,768 bytes.

SHA-256:
`cdcaf451c67738a10d3cb143f5466dc8b659f5570a8ce1fe58e43b387f0cf8d6`.

Apple accepted app submission `d31c4ce7-4904-4edf-92e0-e25d90fa0117` and
DMG submission `ab183b3c-9443-465b-9e82-d93513a27635`. Both stapled tickets and
Gatekeeper assessments passed. The final DMG was mounted read-only, and its
entire app bundle compared equal to the installed app tested above.
