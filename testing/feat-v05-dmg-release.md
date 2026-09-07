# feat/v05-dmg-release — Test Contract

First public macOS download: a signed, notarized `.dmg` that installs Bridge into Applications without a repo checkout.

Locked before implementation.

## Functional Behavior

- **Version.** Workspace, Tauri, npm, and the browser-extension manifest all report `0.5.1`. Handshake `server.version` stays `CARGO_PKG_VERSION` (already wired). Catalog `minimumBridgeVersion` values stay `0.1.0` — that is a compatibility floor, not this release number.
- **DMG target.** `bundle.targets` includes `dmg` (and keeps `app`). `bun run tauri build` writes `src-tauri/target/release/bundle/dmg/Bridge_0.5.1_*.dmg`.
- **Icons.** `src-tauri/icons/` contains the macOS `.icns` / PNG set generated from `assets/bridge-icon.svg`. The Dock and DMG show that mark, not a generic exec icon.
- **Claude sidecar in the bundle.** `prepare:claude-sidecar` copies production sidecar files into `src-tauri/resources/sidecar/claude-agent/` and runs `npm ci` there so `node_modules/@anthropic-ai/claude-agent-sdk` is a real directory (not a bun workspace symlink). `bundle.resources` maps that staging tree onto `Contents/Resources/sidecar/claude-agent/`. `claude_adapter::sidecar_entry` already looks at `../Resources/sidecar/claude-agent/index.mjs`. A copied `.app` must resolve that path without `BRIDGE_CLAUDE_SIDECAR`.
- **Existing sidecars unchanged.** `bridged` and `bridge-browser-host` stay `externalBin` and still go through `prepare:daemon` / `prepare:browser-host`.
- **Signing.** Release builds set `APPLE_SIGNING_IDENTITY` to a Developer ID Application identity. Config does not bake a personal identity, so `tauri dev` keeps working for contributors. Notarization uses App Store Connect API env vars (`APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`) or Apple ID env vars at build time — never committed.
- **Hardened Runtime entitlements.** `bundle.macOS.hardenedRuntime` stays true. `bundle.macOS.entitlements` points at `src-tauri/entitlements.plist`, which grants `com.apple.security.cs.allow-jit`, `allow-unsigned-executable-memory`, and `disable-library-validation`. A notarized app with an empty entitlements blob aborts in WKWebView (`__rust_foreign_exception` on the tao run-loop observer).
- **Docs.** README has a download / install section. CHANGELOG has a 0.5.1 entry.

## Unit Tests

- `bundle_release`: `tauri.conf.json` version is `0.5.1`; `bundle.targets` contains `dmg` and `app`; `bundle.resources` maps `resources/sidecar/claude-agent/` onto destination `sidecar/claude-agent/`; `bundle.macOS.entitlements` is `entitlements.plist` and that file grants the WebKit JIT keys.
- Existing `external_bin_staging` still passes with the two native sidecars.
- Existing `sidecar_entry_honours_explicit_override` still passes.

## Integration / Smoke

- `bun run check`, `bun run test`, `bun run build` green.
- `scripts/prepare-claude-sidecar.sh` leaves a non-symlink `src-tauri/resources/sidecar/claude-agent/node_modules/@anthropic-ai/claude-agent-sdk` for the bundler.

## E2E / Manual (release gate)

1. `scripts/release-dmg.sh` (or `bun run tauri build` with signing + notary env) produces a Developer ID-signed DMG.
2. `codesign -dv --verbose=4` on `Bridge.app` shows a TeamIdentifier, not `Signature=adhoc`. `codesign -d --entitlements -` includes `com.apple.security.cs.allow-jit`.
3. `scripts/notarize-dmg.sh` (or `bun run notarize:dmg`) submits the signed DMG, staples it, and `xcrun stapler validate` succeeds.
4. Install from the DMG into `/Applications`, launch, daemon comes up, Claude adapter finds the bundled sidecar when Node 18+ is on PATH.

## Explicit v0.5 Boundaries

- Apple Silicon host architecture only unless a second target is built later.
- Node 18+ remains required on the user's machine for Claude. Provider CLIs stay optional on PATH.
- Auto-update is out of scope.
- App Store distribution is out of scope.
