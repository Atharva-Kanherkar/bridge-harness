# Bridge in a browser tab

The React app can run in a plain browser tab against a real `bridged`. It is the same UI the desktop app ships, talking the same JSON-RPC contract over the daemon's opt-in WebSocket listener. Nothing is hosted and nothing new is exposed by default: you run the daemon, and the tab connects to it on loopback or through a tunnel you put in front of it.

## Run it

```bash
# 1. A daemon with the WebSocket listener on, allowing the app's origin.
bridged --data-dir ~/bridge-web --listen 127.0.0.1:4319 --allowed-origin http://127.0.0.1:1420

# 2. The app.
bun run dev

# 3. The link that connects a tab to that daemon.
bun run web:link -- --data-dir ~/bridge-web
```

Open the printed link. The tab connects, removes the token from the address bar, and keeps it in `sessionStorage` until the tab closes. A reload stays connected. Pasting a new link into the same tab switches to it.

Without a link the tab runs on mock data, exactly as `bun run dev` always has.

One daemon owns a data directory. To share data with the desktop app, start the app's own `bridged` (the one next to the app executable, `Bridge.app/Contents/MacOS/bridged`) with `--listen` on the desktop's data directory before opening the app. The desktop then attaches to it instead of starting its own. It attaches only to a daemon of its own build: a different build is refused and left running, so a `bridged` built from a checkout won't do. Letting the desktop start its daemon with the listener on is a follow-up.

## How it works

`src/api.ts` has three transports and picks one per window:

| Window | `call()` goes to | `subscribe()` listens on |
| --- | --- | --- |
| Desktop app | Tauri `invoke`, proxied to `bridged` by `src-tauri/src/daemon_host.rs` | Tauri events |
| Browser tab with a daemon link | `WebDaemonClient` in `src/webDaemon.ts`, over WebSocket | the client's notification fan-out |
| Browser tab without one | mock data | mock listeners |

`WebDaemonClient` mirrors `daemon_host.rs` on purpose:

- **Lanes.** The daemon answers a connection's requests one at a time, so the client keeps one socket for notifications and six for calls: four general lanes and two for `github/*`. A slow `gh` read cannot queue a session or health call behind it.
- **Reconnect.** If any socket drops, the pool is rebuilt with backoff (0.5 s doubling to 5 s). After it re-attaches, and whenever the daemon reports `stream-lagged`, the client emits `state-changed`, `adapters-changed` and `stream-lagged` locally, so the UI refetches instead of silently missing events.
- **Errors.** Every rejection is a string, and RPC errors use the same `{"code","kind","message"}` envelope as the desktop host, so `errorMessage()` renders them identically.
- **Refusal is final.** A wrong token (`unauthorized`) or an incompatible protocol stops reconnecting and is shown as the error, never replaced by mock data.

Inside `api.ts`, `hasBackend()` answers "is there a real backend". `isTauri()` is kept only for the desktop shell's own channels: the menu bar, the meter, launch requests, menu commands, and the batched agent stream. The shell's presentation hints (usage overviews, menu-bar settings changed) go through `shellEmit`/`shellListen`, which are Tauri events on the desktop and a local bus in a tab.

## Security model

- The daemon's WebSocket listener is off unless `--listen` is given. It is loopback-only unless `--allow-remote-bind` is also given, and it is plaintext, so put TLS (a reverse proxy or a tunnel) in front of anything beyond loopback.
- A browser upgrade must carry an `Origin` that is exactly on the `--allowed-origin` list. There are no wildcards. Every connection must present the token from `<data-dir>/daemon.token` before anything else.
- The link carries the token in the URL fragment, which browsers never send to a server. The app strips it from the address bar on load and keeps it in `sessionStorage`, which is gone when the tab closes.
- Anyone holding the token can do everything the desktop app can: run agents, write files, open terminals. Treat it like an SSH key.

## What does not work in a tab yet

- **Native pickers.** Choosing a folder asks for a typed path on the daemon's machine.
- **Desktop-only surfaces.** The menu bar, the meter panel, native notifications, the embedded browser, the updater and the quick prompt are not available. External links open in a new tab, and zoom is the browser's own.
- **Notification fan-out.** Every socket in the pool receives every notification, and only the events socket delivers them. The desktop pays the same seven-fold cost over its Unix sockets. A handshake option to opt call sockets out would remove it.
- **Login, TLS, multi-user, hosting.** These are tracked separately (#108). This client is single-user and trusts whoever holds the token.

## Tests

- `src/webDaemon.test.ts`: the client against a fake WebSocket (handshake, id matching, lanes, reconnect, refusal, config parsing).
- `src/api.webDaemon.test.ts`: `api.ts` routing in web mode.
- `src/webDaemon.smoke.test.ts`: against a real `bridged --listen`, skipped unless `BRIDGE_WEB_SMOKE_URL` and `BRIDGE_WEB_SMOKE_TOKEN` are set. See the file header.
- `scripts/test/web-link.test.mjs`: the link script.
