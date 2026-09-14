# Releasing Bridge for macOS

Bridge's production releases are driven by Release Please and GitHub Actions.
The local equivalent of the signed-build stage is `npm run release:dmg`. It
produces a Developer ID signed, notarized DMG and a SHA-256 checksum in
`src-tauri/target/release/bundle/dmg/`. It runs the build and all tests, validates
the signed application, notarizes and staples the application, then creates,
signs, notarizes, and validates the containing disk image. It does not publish a
GitHub release or install the app.

Use a macOS host with Xcode command-line tools, Rust, Node 18 or later, and the
dependencies installed with `bun install --frozen-lockfile`. The resulting DMG
targets the host architecture. The release script uses npm/Node to run commands;
Bun may fail to locate a working directory below a macOS privacy-protected
Downloads folder even when the project directory itself is accessible.

## Automated release flow

Pull request titles must use Conventional Commit syntax, such as `feat:`,
`fix:`, `docs:`, or `chore:`. Merge pull requests with **Squash and merge** so
that the PR title becomes the single release commit Release Please classifies.
For application releases, `fix:` increments the patch version, `feat:` increments
the minor version, and a `!` after the type or scope increments the major
version. The title check accepts the broader Conventional Commit vocabulary so
the history remains structured; Release Please's pinned Node strategy makes the
final release/no-release and version-bump decision.

Before enabling the workflow, configure the repository once:

1. In **Settings > General > Pull Requests**, leave squash merging enabled and
   disable merge commits and rebase merges. Configure squash commits to use the
   PR title with a blank commit message. Release automation fails closed while
   those settings do not make the checked PR title the sole versioning input.
2. In an active `main` branch ruleset, require pull requests and the
   **Conventional PR title** CI check. Do not let the Release Please App bypass
   required CI. Release automation fails closed if the effective rules for
   `main` do not include a pull-request rule.
3. Install the Release Please GitHub App on this repository with read access to
   Administration and read/write access to Contents, Issues, Pull requests, and
   Workflows. Administration read is required to verify the effective `main`
   branch rules before Release Please runs. Workflows write is needed only when
   publishing a recovered tag whose workflow files differ from current `main`.
   Add its client ID and private key using the names below.

Release Please maintains one release PR against `main`, updating its version and
changelog as releasable changes land. Its exact title is
`chore(main): release X.Y.Z`. Merging that release PR creates the matching
`vX.Y.Z` tag and a **draft** GitHub release. The draft is intentionally not
public yet. A downstream job in that same queued `main`-push run calls the
reusable production workflow with the exact successful source SHA. Preflight
checks that protected `main` head, finds the newest matching Release Please tag
in its ancestry, and requires the matching draft and merged Release PR. Every
release job then checks out that exact tag commit before the signing job can
receive credentials. Production jobs run in this order:

1. Build and test Bridge, sign the app and updater artifact, notarize and staple
   the app and DMG, and upload the verified DMG, checksum, updater archive, and
   updater signature for the later jobs.
2. In a separate job with no Apple, updater, provider, or Release Please
   credentials, download that DMG, verify its checksum, and mount it read-only.
   Run `scripts/smoke-packaged-macos-app.py` against the `Bridge.app` inside the
   mounted DMG. The smoke requires the bundled `bridged` daemon to start, accept
   the desktop's authenticated health RPC against an isolated data directory,
   and acknowledge a clean shutdown without leaving its process, socket, or
   ownership lease behind.
3. Only after the mounted-DMG smoke passes, upload the verified assets to the
   existing draft and publish it.

Routine pull-request and `main` CI also runs this lifecycle test against an
ad-hoc-signed app built without credentials. That artifact is diagnostic only:
production always rebuilds from the release tag and tests the signed app inside
the final DMG. An ad-hoc CI bundle must never be promoted, re-signed, or used as
a release input.

## Signing credentials

Release Please authenticates as a GitHub App. Configure:

- Repository variable `RELEASE_PLEASE_APP_CLIENT_ID` with the app's client ID.
- Repository secret `RELEASE_PLEASE_APP_PRIVATE_KEY` with the app's PEM private
  key.

The App should be installed only on the Bridge repository unless it has another
explicit use. Release Please requests Administration read plus Contents, Issues,
and Pull requests read/write permissions from its short-lived installation
token. The final publisher requests only Contents and Workflows write so GitHub
permits both immediate publication and recovery when workflow files changed
after the tag. The primary Release Please job and the build, smoke, and
publication jobs keep their built-in workflow tokens read-only. Production
preflight alone receives Contents write because GitHub hides draft releases from
callers without push access; the protected resolver uses that token only for
read operations.

The production signing job uses these existing repository secrets:

- `APPLE_CERTIFICATE`: base64-encoded Developer ID Application `.p12`.
- `APPLE_CERTIFICATE_PASSWORD`.
- `APPLE_SIGNING_IDENTITY`.
- `APPLE_API_KEY` and `APPLE_API_KEY_P8`.
- `TAURI_SIGNING_PRIVATE_KEY`, used to sign the in-app updater archive.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` when the updater private key is
  password-protected; omit it for an unencrypted key.
- `APPLE_API_ISSUER` only for a Team App Store Connect API key. Individual API
  keys must omit it.

Keep the GitHub App credentials in the Release Please and final publication
jobs, and the Apple/updater credentials in the production build job. The
packaged-app smoke job receives neither credential family and has read-only
repository permissions.

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

- Release Please updates the root app, Tauri, Cargo workspace, browser-extension,
  Safari-extension, Claude-sidecar, and Arch-package versions together. CI also
  validates the generated Cargo, npm, and Bun lockfile entries before any
  release build receives credentials.
- `npm run generate:icons` exports the full platform icon set from
  `assets/bridge-icon.png`, sorts ICNS entries for reproducible output, and checks
  the Doto B icon’s achromatic foreground and dark tile in every macOS PNG
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
- The generated updater archive's Minisign signature must verify against the
  public key committed in `src-tauri/tauri.conf.json`. The build checks it before
  artifact upload and publication checks it again after artifact download, so a
  wrong private-key secret or a changed archive cannot reach users.

To re-submit an already signed DMG containing an already stapled app, pass its
exact path to `npm run notarize:dmg -- /absolute/path/to/Bridge.dmg`. The command
does not guess the most recent file. If the app inside is unstapled, use the full
release command to rebuild the image with a stapled application.

## Retry and manual recovery

A signing, notarization, smoke, or upload failure leaves the GitHub release as a
draft. Fix a transient service or credential problem and re-run the failed jobs
in that workflow run; the retry uses the same Release Please tag and commit.
If that run can no longer be retried, a later successful Release Please run on
`main` can resume the still-current draft: preflight requires the older tag to
be an ancestor of the successful head, but rebuilds and publishes only the
tagged commit. This recovery path refuses a superseded tag.
Production assets may be replaced only by artifacts rebuilt and fully
revalidated from that same tagged commit. Publication remains the final step,
so a partial upload or failed smoke cannot make a release public. Neither the
protected `main`-push workflow nor its reusable production workflow supports
manual dispatch from a branch; this keeps a branch-modified workflow from
receiving release credentials.

The same queued `main`-push run holds the release concurrency group from Release
Please through reusable production, so a later push cannot overtake publication.
An older draft also refuses publication once a newer semantic-version tag exists,
so a delayed retry cannot replace a newer release as `latest`. If the version or
tag is wrong, correct the release PR and let Release Please create the next
release; do not repair it by publishing or retagging by hand.

The production job imports the certificate into a temporary keychain and always
removes the keychain, `.p12`, and `.p8` after the signed artifacts are produced.
Missing inputs fail before signing or publishing.

`npm run test:release` exercises release gates with fixtures, including black
icons, invalid signing and JIT entitlements, credential modes, rejected notary
responses, stale SDK locks, failed staging installs, Release Please orchestration,
and packaged-app lifecycle evidence. It does not need Apple credentials or
contact the notary service.
