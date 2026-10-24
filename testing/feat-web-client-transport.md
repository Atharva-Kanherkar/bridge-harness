# feat/web-client-transport: Test Contract

Bridge web, slice 1 (#838): the React app in a plain browser tab drives a real `bridged` over the WebSocket listener from #815. No protocol change. Hosting, TLS and login are out of scope.

## Functional Behavior

### Modes

- `api.ts` has three transports: Tauri (`__TAURI_INTERNALS__` present), web daemon (a connection is configured), mock (neither). Tauri wins if both are present.
- With no fragment and nothing in `sessionStorage`, a browser tab runs on mock data exactly as before. Every existing vitest suite passes unchanged.
- `hasBackend()` is true in Tauri and web modes. It replaces `isTauri()` at every `api.ts` site that means "a real backend exists".
- `isTauri()` stays only on shell channels: the menu bar (settings, connection, connect OpenCode), the meter panel and tray, reveal-main-window, menu commands, launch requests, layout fullscreen, and `agent-event-batch`.
- The shell's presentation hints that follow a successful backend call (`bridge-provider-usage-overviews`, `bridge-usage-overview`, `bridge-menu-bar-settings-changed`) go through `shellEmit`/`shellListen`: Tauri events on the desktop, a local bus in a tab. The call's result still returns. (Revised during implementation: the main window's `UsageDot` and `App` listen to these, so skipping them would have left the tab's usage display stale.)

### Connecting

- `#bridge-daemon=<ws or wss url>&bridge-token=<token>` is read once at boot, saved to `sessionStorage` under `bridge.webDaemon`, and removed from the address bar with `history.replaceState`. Other fragment content is kept.
- A daemon URL that isn't `ws:` or `wss:` is ignored, and the tab stays in mock mode.
- A link pasted into an open tab (a hash-only change) is stored, stripped, and followed by a reload onto it. A partial or unusual `window` (no storage, no events) is mock mode, not a crash. (Added during implementation, after E2E showed a pasted link was ignored.)
- `bun run web:link -- --data-dir <dir> [--daemon ws://127.0.0.1:4319] [--app http://127.0.0.1:1420]` prints the link, built from `<dir>/daemon.token`. A missing token file exits non-zero with a clear message.

### Wire

- Each socket's first frame is `{"jsonrpc":"2.0","id":N,"method":"protocol/handshake","params":{"authToken":…,"client":{"name":"bridge-web","version":…},"protocolVersion":PROTOCOL_VERSION}}`. No other frame is sent before the handshake answers.
- Responses are matched by `id`. Notifications between responses go to subscribers and never resolve a call.
- A call with no params sends no `params` key. A call with params sends them verbatim.
- An RPC error rejects with the string `{"code":C,"kind":K,"message":M}`, the same as `daemon_host::host_error_envelope`. `kind` is the `ERROR_CODES` name, or null for an unknown code. `errorMessage()` therefore yields `M`.
- A socket that closes rejects its in-flight calls with "The Bridge daemon connection was lost; reconnecting".
- A call made while detached waits up to 10 s for a link, then rejects with "The Bridge daemon is unreachable".
- A request whose serialized frame is over the daemon's 1 MiB limit (`bridged::MAX_FRAME_BYTES`, counted in UTF-8 bytes) is rejected locally and never sent. Other lanes and the events socket are unaffected. A drift test pins the TypeScript limit to the Rust constant. (Review of #839.)

### Pool and lanes

- One events socket plus four general and two GitHub call sockets.
- The first socket must authenticate. The other six are an optimization: those that authenticate are kept, and the pool is laid out from what it got, as `daemon_host::attach`, `split_event_connection` and `lane_range` do. With one socket, events and calls share it. With five or fewer call sockets, GitHub and general calls share them. A daemon at its connection cap with one free slot still serves calls. (Review of #839.)
- `github/*` methods use only the GitHub lanes. Everything else uses only the general lanes. An idle lane is preferred. When all are busy, calls rotate across them.
- With the GitHub lanes all busy, a `health/health` call still completes immediately.

### Back/forward cache

- On `pagehide` the client closes every socket, so a page held in the browser's back/forward cache holds no daemon slots. On `pageshow` with `persisted` it reconnects, reconciles, and fires `onBackendAttached`. A suspended or resumed client never runs two connection loops at once. (Found while verifying the review's connection-cap fix: Chrome kept a navigated-away page's seven sockets open indefinitely.)

### Reconnect and reconciliation

- If any socket in the pool drops, the whole pool is torn down and rebuilt with backoff (0.5 s doubling to 5 s).
- After a successful re-attach, and on every `stream-lagged` notification, the client emits `state-changed`, `adapters-changed`, `stream-lagged` locally, in that order, to subscribers. This matches `daemon_host::emit_reconciliation`.
- An `unauthorized` (2001) or `incompatible_protocol` (2000) handshake rejection stops reconnecting. It surfaces as the error of pending and future calls, and is reported through `onStatus`. It is never silently replaced with mock data.

### Recovery at boot

- `bridgeApi.onBackendAttached` fires whenever the web client attaches, including the first time, but not for a subscriber that arrives already attached. On it, every errored react-query read (health, model setup) is refetched, the state and config reload runs, and a startup error that was only a transport error (unreachable, connection lost) is cleared. A tab opened while the daemon is down for longer than the link wait therefore recovers by itself when the daemon starts. (Review of #839.)

### Agent stream

- In web mode, `onAgentEvent` subscribes to `agent-event` and calls `recordStreamReceipt` and the handler once per frame, in order.

### Shell-only fallbacks

- `openInSystemBrowser` and `openExternalUrl` use `window.open(url, "_blank", "noopener,noreferrer")` outside Tauri. The internal router still runs first.
- If the router has not answered within 250 ms (inside every browser's transient-activation window), a blank tab is reserved with its opener cut. If the router then takes the link, the tab closes. If it declines, the tab navigates to the URL. A fast router never reserves a tab. (Review of #839: a slow `githubStatus` lookup let activation expire, and the browser blocked the new tab.)
- Folder pickers (`createWorkspaceFromFolder`, `connectFolder`, the import "choose folder") ask for a typed path when the backend is the web daemon. Mock mode keeps its demo path. Tauri keeps the native dialog. A cancelled or empty entry does nothing.
- `installZoom` and App's zoom keys do nothing outside Tauri (`ownsZoom()`), so ctrl+wheel and Cmd+/- reach the browser's own zoom. (Revised: gating the keys, not `applyZoom`, is what stops the browser's zoom being swallowed.)

## Unit Tests

`src/webDaemon.test.ts` (fake WebSocket):
- `sends the handshake first with the token and protocol version`
- `matches responses by id with notifications interleaved`
- `omits params for parameterless calls and forwards params verbatim`
- `rejects an rpc error with the host error envelope`
- `rejects in-flight calls when the socket closes`
- `routes github methods to github lanes and prefers an idle lane`
- `a slow github call does not block a general call`
- `rebuilds the pool after a drop and emits reconciliation in order`
- `turns stream-lagged into reconciliation`
- `stops reconnecting after unauthorized and surfaces the message`
- `waits for a link, then rejects as unreachable`
- `readWebDaemonConfig`: fragment parsing, `sessionStorage` persistence, stripping only our keys, rejecting non-ws schemes.

`src/api.webDaemon.test.ts`:
- with a configured client, `bridgeApi.health()` goes through the client, not mock
- `onStateChanged` and `onAgentEvent` receive notifications from the client
- a menu-bar save in web mode returns the saved settings without touching Tauri

`src/externalLinks.test.ts`: outside Tauri, `openInSystemBrowser` calls `window.open` with `noopener,noreferrer`.

`src/zoom.test.ts`: outside Tauri, `installZoom` does not cancel ctrl+wheel.

`scripts/test/web-link.test.mjs`: builds the link from a token file, encodes the components, and fails clearly without a token.

## Integration / Functional Tests

- `src/webDaemon.smoke.test.ts`, skipped unless `BRIDGE_WEB_SMOKE_URL` and `BRIDGE_WEB_SMOKE_TOKEN` are set. Against a real `bridged --listen`, it checks: the handshake succeeds, `health/health` returns `ok`, `state/get_state` returns a workspaces array, a wrong token rejects with `unauthorized`, and a bad param rejects with `invalid_params`. `add_project` on a temp git repo returns the new project, and a second connection sees it. `sessions/create_chat_id` yields a `state-changed` notification on the events socket. (Revised during implementation: `add_project` returns its state rather than publishing `state-changed`. The UI uses the returned state, so the notification check moved to a method that does publish.)

## Smoke Tests

- `bun run build` and `bun run test` are green.
- `bun run dev` with no fragment shows the mock app.

## E2E Tests

Manual, recorded in the PR:
1. `cargo build -p bridged`. Run `bridged --data-dir $TMP --listen 127.0.0.1:4319 --allowed-origin http://127.0.0.1:1420`.
2. `bun run dev`. Open the link from `bun run web:link -- --data-dir $TMP` in a headless browser.
3. The fragment disappears from the address bar, the sidebar shows the daemon's real (empty) state, and health shows real adapters, not the mock ones.
4. Add a project by typed path. It appears, and it also appears through `state/get_state` on a second connection.
5. Kill `bridged` and restart it. The tab reconnects without a reload.
6. Open a tab with a wrong token. The `unauthorized` message is visible, and mock data is not shown.

## Manual / cURL Tests

- `bridged ... --listen` with an origin not on the allowlist is refused at the upgrade. This is already covered by `bridged/tests/remote.rs` and re-checked once from the browser on a different port.
