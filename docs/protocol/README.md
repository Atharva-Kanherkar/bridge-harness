# Bridge protocol

The versioned RPC contract every Bridge client speaks — the Tauri desktop
shell today; the `bridged` daemon, TUI, web, and CI clients as they land. The
single source of truth is the **`bridge-protocol`** crate
(`src-tauri/bridge-protocol/`); everything under `docs/protocol/schemas/` and
`src/protocol/generated/` is generated from it.

```
Regenerate artifacts:   cargo run --manifest-path src-tauri/Cargo.toml -p bridge-protocol --bin generate-protocol-artifacts
Drift is test-enforced: cargo test --manifest-path src-tauri/Cargo.toml -p bridge-protocol
```

## Shape

JSON-RPC 2.0 semantics. Bridge's surface — commands, approvals, cancellation,
bidirectional events — is RPC-shaped, so REST+SSE was considered and rejected.

| Transport | Audience |
| --- | --- |
| Unix-domain socket | Local clients (Tauri shell, TUI, `bridge exec`) — served by `bridged` today |
| WebSocket | Browser and remote clients (follow-on; the auth token and origin rules are designed for it) |
| Tauri invoke/event adapter | The desktop webview; proxied to `bridged` by default, in-process only in embedded fallback |
| Plain HTTP | `/healthz` and `/readyz` only |

## The `bridged` daemon

`bridged` (`src-tauri/bridged/`) is the single local owner of a data
directory's sessions, stores, PTYs, and provider processes:

- **Ownership.** An exclusive OS file lease on `<data_dir>/owner.lock`
  (`bridge_core::ownership`). Exactly one owner per data directory — daemon or
  the desktop app in embedded mode — and the loser fails fast with the
  holder's identity. The lock dies with its process, so `kill -9` needs no
  stale-lock recovery; boot-time recovery handles what the dead owner left.
- **Framing.** Newline-delimited JSON-RPC 2.0 over `<data_dir>/bridged.sock`
  (mode 0600). One frame per line in both directions; notifications
  interleave between responses.
- **Auth.** The handshake must carry `authToken` from `<data_dir>/daemon.token`
  (created 0600 on first start; an existing file is trusted only if it is a
  regular owner-only file holding a 64-hex token — anything else is refused,
  never adopted). A wrong or missing token is rejected with **2001
  `unauthorized`** before the server reveals anything, version included, and
  the whole handshake must complete within a deadline (10s) or the slot is
  reclaimed.
- **Dispatch.** An exhaustive match over the method registry: params are
  validated against the contracted types (wrong casing, unknown fields, and
  out-of-set enum values fail with `invalid_params`), then routed to
  `bridge_core::api` — the same bodies the Tauri shell calls.
- **Limits.** Frames over 1 MiB → `invalid_request`, connection closed.
  Connections beyond the cap → one **2004 `overloaded`** frame, closed.
  Requests per connection are sequential; backpressure is the socket.
- **Events.** One event hub per daemon subscribes to the core bus and fans
  out to per-connection bounded queues. A connection that falls behind is
  told so: it receives a `stream-lagged` notification plus the idempotent
  refetch hints as soon as it drains, and replays durable history via
  `sessions/replay_session_events`. Lag is never silent.
- **Shutdown.** SIGINT/SIGTERM stop the accept loop, drain in-flight
  connections (idle ones close within one poll interval; requests arriving
  after the flag are refused with **2003 `shutting_down`** and their own id),
  then stop live adapters so sessions land recoverable. `kill -9` recovery
  rides boot-time recovery on the next owner.
- **Shipping.** `bridged` is staged by `scripts/prepare-daemon.sh` and ships
  as a Tauri external binary next to the app binary; the bundled browser
  extension is resolved relative to the executable at runtime.
- **Health.** `/healthz` and `/readyz` on `127.0.0.1:4318` by default
  (`--health-addr none` disables). Bind and startup failures are fatal and
  printed — never a silent no-op. Port 4317 stays private and unextended.

```bash
bridged --data-dir ~/Library/Application\ Support/dev.bridge.deck
```

## Clients

**`bridge-client`** (`src-tauri/bridge-client/`) is the shared Rust client:
token handshake, sequential calls with interleaved notifications, and the
recovery rules encoded once — `SessionEventStream` yields a session's durable
events gap-free across disconnects, live-channel lag (`stream-lagged` →
cursor replay), and sequence gaps. The Tauri proxy and TUI build on it.

**The desktop app** runs as a daemon client by default. On startup it
attaches to a `bridged` serving its data directory, or starts the bundled
binary and waits for it (stdout/stderr land in `<data_dir>/bridged.log`).
Every Tauri invoke is proxied generically: the command's registry method is
called with the invoke payload as params (invoke argument names *are* the
wire names; the shell's signature-parity test pins that), and every daemon
notification is re-emitted to the webview with an unchanged name and payload
— the frontend cannot tell which host it is on. A supervisor thread owns the
connection: after a daemon restart it reattaches on its own, and after any
reconnect or `stream-lagged` marker it emits the `state-changed` /
`adapters-changed` refetch hints, so the UI recovers durable history (open
approvals included) from the session forest exactly as the embedded host's
lag path always worked. `BRIDGE_DESKTOP_HOST` selects the host: `auto`
(default: attach → start → fall back to embedded), `daemon` (no embedded
fallback — the acceptance configuration), `embedded` (the in-process runtime,
kept during migration; never concurrent with a daemon thanks to the lease).
`BRIDGE_DAEMON_BIN` overrides which daemon binary is started.

**`bridge exec --json`** is the CI one-shot. It attaches to a running daemon
when one owns the data directory, and otherwise hosts the runtime itself for
exactly the duration of the command — CI never keeps a user daemon alive, and
an embedded desktop owner is reported by identity instead of failing opaquely.

```bash
bridge exec --json --data-dir "$DIR" --method state/get_state            # one call
bridge exec --json --data-dir "$DIR" --harness codex "run the tests"     # one turn, JSONL events
```

Prompt mode requires `--harness` naming an available structured adapter
(checked against `health/health` before any session is created). The
`--timeout` budget covers the whole command — connect, setup calls, and the
event stream; on timeout the turn is interrupted via
`sessions/interrupt_turn`. Turn failure is recognized in both provider
shapes: a `turn.completed` carrying `failed`, and a completion followed by a
trailing failed `error` event (grace-drained, then settled by the session's
own status). Exit codes: 0 success, 1 failure (RPC error / failed turn),
2 usage, 3 timeout.

## Envelope

Requests, responses, and notifications follow JSON-RPC 2.0
(`docs/protocol/schemas/rpc-*.json`), and the contract types enforce the spec
rather than merely describing it:

- `jsonrpc` is the literal `"2.0"` — any other value fails to decode.
- Request ids are client-chosen strings or numbers, echoed back verbatim.
  Numeric ids must stay within JavaScript's safe-integer range
  (±(2^53 − 1)); the wire rejects anything larger, because a browser client
  would silently round it and correlate the response with the wrong request.
- `params`, when present, must be an object or an array. A literal
  `"params": null` is rejected — omit the key instead.
- A response is a **union** of success and failure: exactly one of `result`
  or `error`, never both, never neither. `"result": null` is a valid success
  (commands returning nothing succeed with it).
- A response id may be `null` when the request id could not be recovered —
  the parse-error and invalid-request cases the spec calls out.

## Handshake

The first request on a connection must be **`protocol/handshake`**
(`handshake-request.json` / `handshake-response.json`); any other first
request is answered with `invalid_request`. The server advertises:

- its `protocolVersion` (this document describes **1.0**),
- its identity (`server.name`/`server.version` — the application version), and
- its `capabilities`: the method domains it serves.

**Versioning policy:** a *major* bump is breaking; a *minor* bump is additive
(new methods, notifications, or optional fields). A server accepts a client
when majors match and the client's minor is not newer than the server's.
Incompatible clients are rejected with the stable code **2000
`incompatible_protocol`**, with both versions in `error.data`.

**1.0 is a breaking bump, not a stability claim.** Opening `HarnessId` (below)
widened a value domain that appears in *results*, so a 0.x client — whose
generated `HarnessId` is a closed enum — would handshake successfully and then
fail decoding a snapshot containing a `gemini` session. Widening a result domain
is the "clients must upgrade" case the major is reserved for. 0.x clients are
refused at the handshake rather than served values they cannot decode.

Since 0.6 the request carries an optional `authToken`; hosts serving
remote-capable transports (the daemon) require it, and nothing ever echoes it.

## Methods

`docs/protocol/schemas/methods.json` is the registry. Wire names are
`domain/command`, where `command` is the exact Tauri command name — the
compatibility adapter maps an invoke to its method by name alone, and a test
in the shell crate keeps the registry 1:1 with `generate_handler![...]`.

Domains: `approvals`, `browser`, `completion`, `config`, `health`,
`learning`, `marketplace`, `models`, `projects`, `routing`, `sessions`,
`skills`, `slash`, `state`, `terminal`, `workspaces`.

### Params

**Every** method in the registry carries a `paramsSchema`: a schema file, or an
explicit `null` meaning contracted to take no parameters (a validator rejects
any params sent to it). There is no "not yet contracted" state left — a method
without a params contract fails `cargo test`. See `BridgeMethodParams` in the
generated TypeScript.

Params fields are camelCase, matching Tauri's invoke-argument conversion, and
mirror the command's signature argument for argument — a test in the shell
crate parses each `#[tauri::command]` signature and compares it against the
contracted field names, so renaming an argument without renaming the field
fails the build.

Params objects first contracted in 0.5 **reject unknown fields**. The 19 params
schemas published in 0.4 remain open until the next major version because minor
versions are additive: a 0.5 server must keep accepting every request permitted
by the 0.4 contract. Newly contracted methods have no older schema to preserve,
so an unknown field is a client bug and `invalid_params` is preferable to
silently ignoring it.

Payload types that mirror a `bridge-core` DTO (`CheckRun`, `RouterPreferences`,
`HarnessConfig`, `LearningSchedule`, the browser payloads, and the closed enums
alongside them) exist twice on purpose: the contract does not depend on the
runtime it describes. A test-only module in bridge-core
(`src/protocol_mirror.rs`) is the drift gate — exhaustive matches so a new core
enum variant fails to compile, and JSON round-trips so a renamed or added struct
field fails the test. Input DTOs derive testable serialization too, so optional
field additions cannot slip through a one-way deserialization check.

### Results

Every method's result is contracted or a **documented exception** — never
silently absent. Registry rows carry either `resultSchema` (a schema file,
with a matching type in the generated TypeScript `BridgeMethodResults` map) or
`resultDeferred` (the named core DTO a future slice must mirror). The
aggregate `BridgeState` and `SessionForestSnapshot` trees are fully
contracted, including the nested completion summary; the deferred set is the
remaining domain snapshots (learning runs/state, model setup, the OpenCode
catalog, the browser bridge snapshot, and the marketplace/skill catalogs).
Commands returning no value use the explicit `UnitResult` contract
(`result: null`).

### Harness ids

`HarnessId` is an **open** string naming **which agent** runs a session:

```
HarnessId := [a-z0-9][a-z0-9._-]{0,63}
```

`claude`, `codex`, `opencode`, `shell`, `gemini`, `cline`, … Through 0.8 this
was an enum of the first four; every one of those values is unchanged. Do not
generate a closed union over the values that exist today — an agent installed
from the ACP registry did not exist when your client was compiled, which is
the whole point.

**The id is the agent, never how Bridge runs it.** Claude reached through the
Agent SDK and Claude reached through an ACP shim are the same agent and share
the id `claude`; the transport is Bridge's problem, recorded separately, and
never something a user picks. This is why the live registry's `opencode` entry
and Bridge's own OpenCode adapter are **one** harness with one id and one
session history, rather than two competing products. It also means a bespoke
adapter can replace a generic one later without renaming anything or migrating
a single session.

Whether an id is *runnable* is a separate question from whether it is *valid*.
A well-formed id Bridge has no adapter for parses fine and fails at start with
an error naming the harness.

**Params and results use different types, and the schemas say so.** A
parameter is `HarnessId`, whose schema carries the `pattern` above — a
malformed id is `invalid_params`. A result carries `StoredHarnessId`, an
**unconstrained** string: a session persisted by a newer Bridge, or one whose
agent was uninstalled, still reports the id it was stored with, so history
does not disappear because an agent was removed. Publishing the strict pattern
on the result side would let `state/get_state` return a document that fails
its own contract.

So: read any string from `sessions[].harness`; send back only one that matches
the grammar. An id outside the grammar is readable, never actionable.

## Cancellation

`$/cancel` is a **notification** (`cancel-params.json`) naming the in-flight
request id. Cancellation is best-effort, LSP-style: the cancelled request
still receives a response — its result if it won the race, otherwise error
**2002 `cancelled`**.

On the daemon's socket transport, requests are handled sequentially per
connection, so a `$/cancel` is only ever read after the request it names has
completed: it is consumed as a valid no-op, exactly as best-effort allows.
Sending `$/cancel` as a *request* is answered with `invalid_request` — the
daemon will not claim a cancellation that cannot have happened. The
domain-level interrupt for a running turn is `sessions/interrupt_turn`.

## Errors

Codes are the contract; message text is not. Never reuse or renumber. The
full registry with summaries is `docs/protocol/schemas/error-codes.json`.

| Range | Meaning |
| --- | --- |
| `-32700..-32600` | JSON-RPC envelope errors (spec-reserved values) |
| `1000..1999` | Runtime errors — variant-for-variant with `bridge_core::BridgeError` |
| `2000..2999` | Protocol lifecycle: `incompatible_protocol`, `unauthorized`, `cancelled`, `shutting_down` |

## Notifications (events)

`docs/protocol/schemas/notifications.json` is the registry: every
notification a host pushes, marked **durable** or **transient**. Wire names
are the Tauri event names, so the compatibility adapter forwards them
unchanged.

| Notification | Delivery | Meaning |
| --- | --- | --- |
| `agent-event` | mixed | Persisted entries carry a positive per-session `sequence`; sequence-zero streaming frames are transient |
| `state-changed` | transient | Refetch hint: re-read the state snapshot |
| `adapters-changed` | transient | Refetch hint: re-read adapter availability |
| `learning-job-changed` | transient | Refetch hint carrying the changed run/state |
| `session-output` | transient | Terminal bytes; worthless once stale |
| `account-usage` | transient | Provider usage tick for the ambient meter |
| `stream-lagged` | transient | Host-synthesized: this connection's live channel dropped events (`{"missed": n}`); refetch hints follow, replay durable history from your cursors |

The delivery rules are contract: the session forest (SQLite) is the
authoritative history; the live channel is bounded and notify-only — it
drops the oldest events when a receiver lags and must never be treated as
the source of truth. Durable events are published **only after their DB
transaction commits** and expose the cursor needed to reconcile from the
session forest after a disconnect or lag. Sequence-zero `agent-event` frames
are transient and never replayed. Refetch hints are coalesced and re-emitted
after live-channel lag so clients converge even when the original hint was
evicted. The Tauri compatibility UI also polls the forest; the dedicated
replay method exists as **`sessions/replay_session_events`** — pass the last
seen durable cursor (`afterSequence`) and receive the next page of missed
durable events in order, no gaps, no duplicates. `limit` is optional (default
500, maximum 1000); continue from the last returned sequence until the method
returns fewer than the requested limit. Transient and sequence-zero frames are
never replayed. To close the subscribe-before-replay race, clients discard live
durable events at or below the highest sequence returned by replay, then process
newer live events normally. The daemon brings the remote transport for it;
in-process hosts call it like any other method.

## Generated client types

`src/protocol/generated/protocol.ts` is generated **from the JSON Schemas**
(one source of truth) and type-checked by `tsc` in `bun run check`/`build`.
It exports the envelope and handshake interfaces, the `BridgeMethod` string
union, the `BRIDGE_METHODS` table, `ERROR_CODES`, and `PROTOCOL_VERSION`. The
frontend migrates onto these types as the compatibility adapter lands.
