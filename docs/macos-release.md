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

A version tag must match `tauri.conf.json`. Only a successful, fully verified tag
build publishes a release. Asset uploads complete in a draft before it becomes
public; existing releases are not overwritten. A manual run from a branch
retains the verified artifact without publishing. Missing secrets fail the job
before building or publishing anything.

`npm run test:release` exercises release gates with fixtures, including black
icons, invalid signing and JIT entitlements, credential modes, rejected notary
responses, stale SDK locks, and failed staging installs. It does not need Apple
credentials or contact the notary service.
