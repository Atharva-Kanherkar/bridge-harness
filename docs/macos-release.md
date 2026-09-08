# Releasing Bridge for macOS

The public release command is `npm run release:dmg`. It produces a Developer ID
signed, notarized DMG and a SHA-256 checksum in
`src-tauri/target/release/bundle/dmg/`. It runs the build and all tests, validates
the signed application, notarizes and staples the application, then creates,
signs, notarizes, and validates the containing disk image. It does not publish a
GitHub release or install the app.

Use a macOS host with Xcode command-line tools, Rust, Node 18 or later, and the
dependencies installed with `bun install --frozen-lockfile`. The resulting DMG
targets the host architecture. The release script uses npm/Node to run commands;
Bun may fail to locate a working directory below a macOS privacy-protected
Downloads folder even when the project directory itself is accessible.

## Signing credentials

A public release needs a valid **Developer ID Application** certificate and its
private key in the keychain. An Apple Development certificate or an ad-hoc
signature cannot pass the release gate. The script discovers a Developer ID
Application identity automatically, or uses `APPLE_SIGNING_IDENTITY` when set.

Store notarization credentials outside the repository, either as exported
environment variables or shell assignments in `~/.bridge-release/env`. Set that
file's permissions to `600`. An alternate file can be selected with
`BRIDGE_RELEASE_ENV`. Plain assignments in this file are exported to child
processes automatically; do not run the release scripts with shell tracing.

For an App Store Connect API key:

```sh
APPLE_SIGNING_IDENTITY='Developer ID Application: Your Name (TEAMID)'
APPLE_API_KEY='YOUR_KEY_ID'
APPLE_API_KEY_PATH='/absolute/path/to/AuthKey.p8'
# Team API keys also require this. Omit it for an Individual API key.
APPLE_API_ISSUER='YOUR_TEAM_ISSUER_UUID'
```

Alternatively, use an Apple ID and an app-specific password:

```sh
APPLE_SIGNING_IDENTITY='Developer ID Application: Your Name (TEAMID)'
APPLE_ID='developer@example.com'
APPLE_PASSWORD='YOUR_APP_SPECIFIC_PASSWORD'
APPLE_TEAM_ID='YOUR_TEAM_ID'
```

Use one complete credential family. Partially configured API credentials fail
validation instead of silently falling back to another account.

## What is verified

- `npm run generate:icons` exports the full platform icon set from
  `assets/bridge-icon.svg`, sorts ICNS entries for reproducible output, and checks
  the Span icon's mint deck, off-white supports, and dark tile in every macOS PNG
  size and embedded ICNS PNG. The Tauri build hook runs this automatically.
- The app must pass strict code-signature verification and have a Developer ID
  authority, a TeamIdentifier, Hardened Runtime, and the actual entitlement
  values from `src-tauri/entitlements.plist`. A key present with a false value is
  insufficient.
- Bundle version and identifier must match this checkout. Claude sidecar source
  files must be current, its SDK must be a real directory at the locked version,
  and both native sidecar executables must exist. The SDK's platform-specific
  native Claude executable must also exist at the locked version, retain its
  executable permissions, and independently pass signature, Hardened Runtime,
  and JIT checks. Its upstream vendor signature is preserved; its signing team
  does not have to match Bridge's team.
- Claude dependency staging only reuses `node_modules` when package metadata,
  the lockfile, host platform/architecture, and the installed dependency tree
  agree. Failed installs preserve the last complete staging tree.
- Apple must return `Accepted`; a successful notarytool process alone is
  insufficient. Both the app and the DMG receive validated stapled tickets and
  pass Gatekeeper assessment. The DMG is mounted read-only to verify its actual
  application, rather than an adjacent app from a different build.

To re-submit an already signed DMG containing an already stapled app, pass its
exact path to `npm run notarize:dmg -- /absolute/path/to/Bridge.dmg`. The command
does not guess the most recent file. If the app inside is unstapled, use the full
release command to rebuild the image with a stapled application.

## GitHub Actions

`release-macos.yml` uses the same release script. Configure repository secrets
`APPLE_CERTIFICATE` (base64 Developer ID Application `.p12`),
`APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_API_KEY`, and
`APPLE_API_KEY_P8`, plus `APPLE_API_ISSUER` for Team API keys. The job imports the
certificate into a temporary keychain and removes the credentials afterwards.

Every push to `main` (normally a merged PR) queues a public release. No manual
version tag, local Mac, or separate release approval is needed. Protect `main`
with PR review and required CI checks so only reviewed changes reach this path.
Manual dispatch is also available on `main` to retry a failed release; dispatches
on other branches are skipped. To retry an older failed merge, rerun its original
workflow run rather than dispatching a new run at the current main commit.

The workflow checks out the event's exact commit and uses
`scripts/ci-release.mjs prepare` to select the next unused stable version. It
increments the patch component beyond all existing stable version tags, or honors
a higher version deliberately supplied in the source (for a minor/major release).
App, Rust workspace, sidecar, browser extensions, and their relevant lock metadata
are updated together without refreshing dependencies. A detached release commit
records the source SHA; only its tag is pushed. `main` is never rewritten by the
release bot. Inspect the release tag to see the exact versioned build source.

Releases share a concurrency queue (`queue: max`, up to 100 waiting runs). Each
queued run keeps its own source commit and runs the full release script, including
frontend, sidecar, release-script, and Rust tests, before publication. Missing
secrets, failed tests, rejected notarization, invalid signatures, or a checksum
mismatch stop publication. The DMG is built for the `macos-15` runner architecture;
this workflow does not produce a universal binary.

The verified tag and draft release are created only after the release gates pass.
Both DMG and checksum upload before the draft becomes public. Retries reuse the
same tagged release commit and can replace incomplete **draft** uploads; already
public releases are left untouched. Retrying an older draft never moves the
Latest label backwards. A failure before tagging reserves no version. All of this
happens in one workflow because tags pushed using `GITHUB_TOKEN` do not start a
second push-triggered workflow. `contents: write` is scoped to the release job;
no personal access token is needed.

The workflow becomes active when this configuration **and the hardened release
scripts it calls** reach `main`. Keep these changes together when integrating the
release branch; do not enable automatic publication using the older packaging
implementation. This publishes new downloads to GitHub Releases; installing
updates inside an already installed app requires a separate updater integration.

`npm run test:release` exercises release gates with fixtures, including black
icons, invalid signing and JIT entitlements, credential modes, rejected notary
responses, stale SDK locks, failed staging installs, automatic version allocation,
release retries, tag collisions, and interrupted uploads. It does not need Apple
credentials or contact the notary service.
