# Bridge protocol

The versioned RPC contract every Bridge client speaks — the Tauri desktop
shell today; the `bridged` daemon, TUI, web, and CI clients as they land. The
single source of truth is the **`bridge-protocol`** crate
(`src-tauri/bridge-protocol/`); everything under `docs/protocol/schemas/` and
`src/protocol/generated/` is generated from it.

```
Regenerate artifacts:  cargo run -p bridge-protocol --bin generate-protocol-artifacts
Drift is test-enforced: cargo test -p bridge-protocol
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
(`docs/protocol/schemas/rpc-*.json`). Request ids are client-chosen strings or
numbers and are echoed back verbatim. A response carries exactly one of
`result` or `error`.

## Handshake

The first request on a connection must be **`protocol/handshake`**
(`handshake-request.json` / `handshake-response.json`); any other first
request is answered with `invalid_request`. The server advertises:

- its `protocolVersion` (this document describes **0.1**),
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
current serde signature.

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

Typed notification kinds land with the event-semantics work: the session
forest remains the authoritative durable history with per-event cursors;
live notifications are notify-only and lossy under lag, with a replay RPC to
catch up after disconnect. Terminal bytes and usage ticks are explicitly
transient. The contract here will name each notification and mark it durable
or transient.

## Generated client types

`src/protocol/generated/protocol.ts` is generated **from the JSON Schemas**
(one source of truth) and type-checked by `tsc` in `bun run check`/`build`.
It exports the envelope and handshake interfaces, the `BridgeMethod` string
union, the `BRIDGE_METHODS` table, `ERROR_CODES`, and `PROTOCOL_VERSION`. The
frontend migrates onto these types as the compatibility adapter lands.
