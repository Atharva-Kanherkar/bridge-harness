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

## Signed public artifact

The signing blocker was resolved on September 8. A new Developer ID Application
certificate was issued against an encrypted key generated on this Mac, imported
into the login keychain, and verified by actually signing a native binary.
The existing notarization API key authenticated with its retrieved Issuer ID.
All credentials and the encrypted PKCS#12 backup are outside the repository.

The complete release gate passed with `RUST_TEST_THREADS=4`. An earlier run
failed one Grok fixture probe that passed immediately in isolation; no runtime
cause was established. Three other probe tests were found to silently skip a
misplaced fixture. Their test-only correction uses isolated fixture wrappers;
all 18 Grok tests then passed, including those three cases.

The signed app passed fresh setup, repeated native zoom, fullscreen, and Quit
(exit status 0), without the ad-hoc signing warning or native exceptions.

Apple notarization results:

- App ZIP: `7c11b0bb-15a8-4057-a22a-fe4ae1ec73de` — Accepted.
- DMG: `b1b4fded-27bb-4832-b26c-68d7c3248d88` — Accepted.
- Both tickets were stapled and validated. Gatekeeper accepted the app and DMG
  with `source=Notarized Developer ID`.
- The DMG was mounted read-only and its bundled app passed the strict public
  signature, entitlement, icon, version, and sidecar checks.

Public artifact:
`src-tauri/target/release/bundle/dmg/Bridge_0.5.2_aarch64.dmg`.

SHA-256:
`e4c995d9fda2af250381066aeed175ae37aa562ad77855cad967a94d917e852c`.

## Downloaded release installed on the user's Mac

On September 8, the published GitHub asset was downloaded again through Safari.
Its 147,529,174 bytes matched the public SHA-256 above. Safari's quarantine
attribute was preserved through Finder copying and installation to
`/Applications/Bridge.app`. Gatekeeper accepted both the downloaded DMG and the
installed app. The icon was visibly legible in Finder, and installed executable
bytes matched the mounted download. The disk image was ejected before launch.

The installed copy used the existing default application data, backed up before
the test. It ran for about nine minutes through native zoom and fullscreen,
project navigation, a live Codex repository-read task, and a follow-up that
correctly remembered the file. A duplicate launch exited with status 0 without
disturbing the primary process. Normal Quit stopped the desktop and daemon;
the installed copy reopened with its conversation intact. No new native crash
report or diagnostic exception was recorded.

The restart check exposed a separate lifecycle defect: otherwise successful
chats were labelled failed on reopening because normal shutdown left durable
provider-process ownership behind. Recovery emitted
`Bridge restarted with a tracked provider process; restoration is required before continuation`.
Sending another message restored the session and correctly recalled the file,
but the false failure classification requires the 0.5.3 shutdown fix. These
checks do not establish that every provider or long-running workload is healthy.
