# fix-cursor-login-reprobe — Test Contract

## Functional Behavior

- Signing in through Bridge's provider login pane makes the provider usable without restarting Bridge.
- Cursor proves availability with a handshake and records the answer keyed on the binary's path and version. A sign-in changes neither, so when the login terminal for a provider exits, the host asks that provider's adapter to re-establish availability instead of serving the recorded probe.
- The refresh is non-blocking and unconditional: a cancelled or failed login re-probes to the same answer, and the fresh result announces itself through the same adapters-changed hint the startup discovery uses, so the usage widget and model picker update on their own.
- The recorded answer stays served until the fresh one replaces it; no reader ever observes an emptied probe mid-refresh.
- Adapters that read availability fresh on every descriptor treat the refresh as a no-op, and a provider with no structured adapter is ignored rather than an error.

## Unit Tests

- Rust cursor_adapter test proves a refresh probes again for a byte-identical build: a recorded needs-sign-in outcome whose path and version still describe the executable is replaced by a fresh successful handshake against the scripted CLI.
- Rust adapters test proves registry refresh routing: the named adapter's override runs, the default is a no-op, and an unknown id does nothing.

## Integration / Functional Tests

- The provider-login exit path calls the registry refresh before publishing the terminal-exited event, for whatever provider the pane ran.
- Full bridge-core suite remains green.

## Smoke Tests

- `bun run check` succeeds.
- `bun run test` succeeds.

## E2E Tests

- Covered by the scripted CLI fixture rather than a live vendor login: a real OAuth round trip needs a browser and an account, and the seam under test is the probe cache, which the fixture exercises over the real handshake.

## Manual / cURL Tests

- With a signed-out cursor-agent, open the usage widget, click Sign in, complete the browser flow, and confirm the tile flips to signed in with real models within a few seconds of the pane closing — no app restart.
- Cancel a sign-in midway and confirm the tile still reads not signed in and the app stays healthy.
- No cURL test is applicable: this is a Tauri invoke and PTY flow rather than an HTTP endpoint.
