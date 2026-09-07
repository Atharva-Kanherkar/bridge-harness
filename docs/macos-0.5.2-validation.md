# Bridge 0.5.2 macOS validation

Validated on macOS 26.6.2, Apple Silicon, on September 7–8, 2026.
The tested implementation is commit `5ac4c77` on `codex/fix-macos-dmg-release`.

## Automated checks

- Frontend production build passed.
- Complete root test command passed: 127 frontend suites / 1,644 tests,
  2,094 Rust tests (13 existing ignored cases), sidecar tests, 14 release-script
  tests, and 12 crash-analyzer tests.
- The documented Bun scripts were invoked through `npm run` on this Mac
  because Bun could not resolve the protected Downloads working directory.
- Release native build used Tauri's `tauri/custom-protocol` feature, then
  bundled the app with the rebuilt daemon and staged Claude SDK.
- The app's ad-hoc signature passes `codesign --verify --deep --strict`.
  The icon validator decoded every required macOS ICNS representation.

## Actual application checks

The test used a separate short `/tmp/bridge-verify-*` data directory.

- Fresh setup saved recommended model profiles and opened the main screen.
- Setup remained complete after quit and relaunch.
- Repeated native zoom and fullscreen transitions did not reproduce the
  earlier parent-view deallocation exception.
- The app stayed running for over six minutes through the checks. This is a
  bounded smoke test, not a long-running provider workload or proof that all
  possible historical crashes are eliminated.
- A duplicate launch exited with status 0 while the primary app stayed alive.
- Native Quit succeeded. Relaunch followed by the close button exited with
  status 0, exercising window destruction and desktop-lease release.
- No native exception or panic was recorded in either final smoke-test log.

Tested app executable SHA-256:
`402d8f2788a6ebdbf766d89acd037cb6daf3070115d7f581d0196a27111d4043`.

## Disk image and public-release status

The local-only artifact is
`.generated/macos-0.5.2-smoke/Bridge_0.5.2_aarch64-local-test.dmg`.
It was mounted read-only; the app signature, icon, Applications symlink,
version, and exact desktop/daemon/browser-host executable bytes were verified.

Local DMG SHA-256:
`36ce7c0d8c2ea908e90b343df6cea1ccb86a69e319c8608fd0d9e3ba676e277f`.

This DMG contains an ad-hoc signed app and is **not a public release**. The supplied
Developer ID certificate imports and validates, but its matching private key
is absent from this Mac's keychain. The supplied notarization API key also
returned HTTP 401 without issuer information. Public signing, Apple
notarization, and GitHub publication remain pending those credentials. The
strict release script rejects this local test artifact for public release.
