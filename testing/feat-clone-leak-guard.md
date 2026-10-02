# feat/clone-leak-guard — test contract

Locked before implementation. Part 2 of the browser-clones epic (#751), stacked
on part 1 (#750). Two-layer containment so nothing a clone runs can carry an
approved session to a host that does not own it. Out of scope on purpose: where
the allowed hosts and secrets come from (part 3), any UI (part 4), Safari.

## Shape of the thing

`src-tauri/bridge-core/src/browser_clone_guard.rs` holds one shared
`GuardState` (allowed hosts + owned secrets + a block audit) and the two layers
that read it:

- **Layer 1, the request checker.** `Fetch.enable` at the browser level pauses
  every request; `fetch_verdict` scans the URL, headers, and body for any
  approved secret in plain, percent, base64 (std and url, padded and not), or
  hex form, and fails the request when it carries a secret to a host that does
  not own it.
- **Layer 2, the egress proxy.** `EgressProxy` is a local forward proxy the
  clone is launched behind (`--proxy-server` + `--proxy-bypass-list=<-loopback>`).
  It opens connections only to allowed hosts. It exists because CDP `Fetch`
  never sees WebSockets, and because a browser behind a proxy resolves no DNS
  of its own.

Wiring: `CloneConfig.guarded` launches the clone behind the proxy and arms the
request checker after ready. `CloneSupervisor::clone_guard(id)` hands back the
shared state so a caller (part 3) can allow hosts and register secrets. The
guard state starts empty: a guarded clone is default-deny.

## 1. The value scanner — `browser_clone_guard::tests`

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | A secret to its owner is allowed | `request_verdict` allows the owner host (and subdomains) |
| 1.2 | A secret to another host is blocked | a foreign host carrying the value is a `Block` |
| 1.3 | Encodings are caught | base64 (std/url, padded/unpadded), hex (upper/lower), percent — each blocks |
| 1.4 | No false positive | ordinary text to a foreign host is allowed |
| 1.5 | Short secrets never match | a value under 8 chars produces no encodings to scan for |

## 2. The request checker — `browser_clone_guard::tests`

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | A leak becomes `Fetch.failRequest` | `fetch_verdict` returns `failRequest` + `BlockedByClient`, and records a block whose host is the destination and whose reason omits the value |
| 2.2 | The owner is continued | a request to the owning host returns `Fetch.continueRequest` and records nothing |

## 3. The egress proxy — `browser_clone_guard::tests`

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | Host matching covers subdomains only | `granted.test` allows `www.granted.test`, rejects `granted.test.evil.com` and `notgranted.test` |
| 3.2 | A disallowed CONNECT is refused | `CONNECT evil.test:443` gets `403`, and an `egress-proxy` block is recorded for the host |
| 3.3 | An allowed CONNECT tunnels | `CONNECT` to an allowed origin returns `200 Connection Established` and pipes bytes both ways |
| 3.4 | Launch args keep loopback in scope | args carry `--proxy-server=…<port>` and `--proxy-bypass-list=<-loopback>` |

## 4. Real browser — `browser_clone::tests::live` (env-gated)

`the_leak_guard_blocks_exfiltration_but_allows_the_task`. Runs only under
`BRIDGE_CLONE_LIVE=1`; `BRIDGE_CLONE_BROWSER` points it at Brave or Chrome for
Testing. A guarded clone allows `127.0.0.1` and registers a secret owned by a
host it never contacts.

| # | Behaviour | Assertion |
|---|---|---|
| 4.1 | The allowed origin loads through the proxy | the local page records a `/` hit |
| 4.2 | Layer 1 fails a same-origin request carrying the foreign secret | a `request-checker` block is recorded and the collector never sees `/collect`, even though `127.0.0.1` is allowed |
| 4.3 | Layer 2 refuses a WebSocket to a disallowed host | an `egress-proxy` block for `blocked.invalid` is recorded (the request checker never sees the WebSocket) |
| 4.4 | No value leaks into the audit | no block reason contains the secret |

Mutation-checked: forcing `request_verdict` to always allow makes 4.2 fail
(the collector receives `/collect`), so the test is not a false pass.

## 5. Repo gates

- `bun run check`, `cargo test -p bridge-core --lib`, and `bun run build` are
  green. The four `terminal_workspace`/`api` PTY failures are pre-existing on
  `origin/main` on this machine and untouched here.
- No protocol method and no generated artifact changes. Everything new is
  behind `#[cfg(target_os = "macos")]`.

## Findings

- **CDP `Fetch` is blind to WebSockets.** Verified during research: a worker
  `WebSocket` carrying the secret slips past the request checker entirely. This
  is the whole reason layer 2 exists; 4.3 is its regression guard.
- **The verdict handler must not wait.** It runs on the pipe's reader thread, so
  it sends its `Fetch.continueRequest`/`failRequest` with a fire-and-forget
  write and never blocks on a reply the same thread would have to deliver. It
  holds a `Weak` to the pipe so it never keeps a destroyed clone alive.

Explicitly **not** changed: the clone process, RAM disk, and lifecycle from
part 1 (its tests still pass unchanged); the attached-tab bridge; the protocol.
