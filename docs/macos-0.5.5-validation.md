# Bridge 0.5.5 macOS validation

Validated on the user's Apple Silicon Mac on September 8, 2026. Build source:
`4ae6579`. This patch adds the Span icon to 0.5.4; native runtime and frontend
behavior are unchanged.

## Automated checks

- Production frontend build passed.
- All 140 frontend suites / 1,848 tests passed.
- All 2,270 Rust tests passed, with 13 existing ignored cases.
- Sidecar and release-script tests passed, including the new Span export checks.
- The app and its bundled helpers passed signature, Hardened Runtime, JIT
  entitlement, version, SDK dependency, and icon checks.

## Installed application

The previous 0.5.4 app and application data were backed up before installing the
notarized candidate at `/Applications/Bridge.app`. The About window visibly
showed version 0.5.5 and the mint-and-white Span icon. The saved smoke-test
conversation rendered correctly. Native fullscreen entry and exit passed.
Normal Quit stopped both the desktop and daemon, and reopening rendered the
application again. No new Bridge or WebKit crash report appeared during these
bounded checks.

The final DMG was mounted read-only, and its entire app bundle compared equal
to the installed application tested above. This icon-only patch did not repeat
live provider or long-workload tests; the 0.5.4 provider test scope is recorded
in [the preceding validation report](macos-0.5.4-validation.md).

## Public artifact

`Bridge_0.5.5_aarch64.dmg` — 146,068,730 bytes.

SHA-256:
`a11d2916aa8e189192e90e84f4a8f76e6e32f5a29032911a5372b7fbf1af3a3d`.

Apple accepted app submission `7c2897ee-79c9-42c9-b950-be3b2ae14882` and
DMG submission `b9decc74-88cf-401c-a6f5-74d8ee6da1b8`. Both stapled tickets
and Gatekeeper assessments passed.
