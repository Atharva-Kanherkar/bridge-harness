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
| Unix-domain socket | Local clients (Tauri shell, TUI, `bridge exec`) |
| WebSocket | Browser and remote clients |
| Tauri invoke/event adapter | Migration compatibility while the shell still hosts the runtime in-process |
| Plain HTTP | `/healthz` and `/readyz` only |

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

- its `protocolVersion` (this document describes **0.3**),
- its identity (`server.name`/`server.version` — the application version), and
- its `capabilities`: the method domains it serves.

**Versioning policy:** a *major* bump is breaking; a *minor* bump is additive
(new methods, notifications, or optional fields). A server accepts a client
when majors match and the client's minor is not newer than the server's.
Incompatible clients are rejected with the stable code **2000
`incompatible_protocol`**, with both versions in `error.data`.

## Methods

`docs/protocol/schemas/methods.json` is the registry. Wire names are
`domain/command`, where `command` is the exact Tauri command name — the
compatibility adapter maps an invoke to its method by name alone, and a test
in the shell crate keeps the registry 1:1 with `generate_handler![...]`.

Domains: `approvals`, `browser`, `completion`, `config`, `health`,
`learning`, `marketplace`, `models`, `projects`, `routing`, `sessions`,
`skills`, `slash`, `state`, `terminal`, `workspaces`.

Per-method typed params/results land domain by domain as command bodies move
onto `BridgeCore`; until a domain is typed, params mirror the command's
current serde signature. Typed so far: **projects**, **workspaces**, and the
**entire sessions domain** — with the live-turn extraction, `start_session`,
`start_chat`, `prepare_turn`, `send_turn`, and `stop_session` joined the
management and control methods, and the test-enforced pending list is empty.
Parameterless methods are contracted with `"paramsSchema": null` (a
validator rejects any params), distinct from an absent key which means
not-yet-contracted. See `BridgeMethodParams` / `BridgeMethodResults` in the
generated TypeScript.
Methods returning the aggregate `BridgeState` snapshot keep untyped results
until the snapshot DTO itself is contracted — that is its own slice. Commands
returning no value use the explicit `UnitResult` contract (`result: null`).

## Cancellation

`$/cancel` is a **notification** (`cancel-params.json`) naming the in-flight
request id. Cancellation is best-effort, LSP-style: the cancelled request
still receives a response — its result if it won the race, otherwise error
**2002 `cancelled`**.

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
