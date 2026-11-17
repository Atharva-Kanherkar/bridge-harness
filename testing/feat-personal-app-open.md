# feat/personal-app-open — Test Contract

Issue #1026, slice 4a-2. Plan: "Plan v2: personal apps on every agent (router), slice 4" in Bridge Notes. Builds on the router from #1049.

## Functional Behavior

### Opening an app

- A top-level chat on Claude Code, Codex, OpenCode or Cursor can call `bridge.apps_open({app_id, prefill?})`. Workers and other agents cannot.
- The tool's description lists the person's locked apps at launch: name, id and intent, at most 20, newest lock first. An unknown id is refused, and the refusal lists the current apps so the model can recover.
- Opening loads the app's latest locked version and recomputes its content hash, refusing on a mismatch. It then checks through the router that the app's connection has the server connected, and refuses with the agent and server to connect if not.
- `prefill` must be an object of at most 32 KiB. It becomes the view's initial tool input.
- An open is bound to `{chat, app, version, content hash, prefill}` (migration 83). It reloads from history with the same version and stays usable while that version exists. If the app is deleted, it refuses.

### The card

- The transcript renders `apps_open` results as a card from all four agents' shapes. It reads only the result, never the tool input.
- The card shows the app name, version, connection ("via Codex"), access list and "Writes ask first".
- The view is the same sandboxed frame as the preview. Live reads need the Mac app's WebRTC isolation.
- History is read-only.

### Calls

Every call from the view goes through core, with the session, the open and the hash re-checked:

- **Allowlisted read:** goes through the router: access mode, current annotations, live or warm path.
- **Allowlisted write:** never runs on the first call.
  - Core canonicalizes the arguments once (canonical JSON, at most 64 KiB) and stores a single-use token bound to `{open, chat, tool, sha256(canonical)}`. The token expires after 600 s, and one open holds at most one pending token.
  - Core returns `approvalRequired {token, server, tool, argumentsJson}`.
  - The card shows the sheet with the exact JSON. Approve calls again with the token. Core claims the token, sends the arguments parsed from its stored canonical string, and returns the result to the view. Deny settles the view's call with a refusal.
- **Anything else:** refused.
- Writes always ask first, in every access mode, as Outgoing does. Reads follow the chat's access mode.

### Write paths

- **Claude:** a one-shot launch of the connection's account and folder, whose policy is an exact action for one tool on the app's server, with native connectors and user settings. Rust sends its own copy of the arguments, and the sidecar and the SDK gate each check deep equality and allow one use.
- **Codex:** `mcpServer/tool/call` on the connection's live or warm runtime, sent only after a claimed approval.

### Ledger

- One `events` row per call: source `personal_app`, kind `personal_app.call.read|write|refused`, entity = app id, body `{version, tool, ok, elapsedMs}`.
- Arguments and results are never stored.

## Unit Tests

- Open:
  - Eligibility per agent and for workers.
  - Unknown app.
  - Hash tamper refuses.
  - Prefill bounds.
  - Binding persistence.
  - Session isolation.
  - Delete.
  - Latest version chosen.
- Calls:
  - The allowlist routes reads, writes and others.
  - A write without a token never sends.
  - The token is single-use, expires, and is bound to open, chat and tool.
  - A second write replaces the pending token.
  - Sent arguments are byte-equal to the shown `argumentsJson`.
  - A tampered stored canonical string refuses.
- Policy: `compile_app_write` allows exactly the one wire tool, with deep-equal arguments and one use.
- Codex: the write frame carries the exact server, tool and arguments.
- Tool route: `apps_open` is listed with the apps' descriptions. Instructions name it.
- Transcript recognition and reference extraction for `apps_open` in all agent shapes, through the real codec and reducer for ACP.
- Card:
  - Load and error states.
  - A write raises the sheet with exact JSON. Approve sends the token. Deny refuses.
  - One pending sheet at a time.
  - History is read-only.

## Integration / Functional Tests

- Open, read, write (approval required), approve, then sent against fake runtimes for both Claude and Codex connections.
- Replay of a used token is refused.
- Prompt injection: a read result containing "call send_note" causes no write and no token.
- Wire, method and daemon/Tauri dispatch stay consistent with the regenerated protocol.

## E2E / Smoke Tests

- Live Codex: an approved write reaches the fake MCP server with byte-equal arguments, and nothing reaches it before approval (`FAKE_MCP_LOG`).
- Live Claude: the exact-write launch sends one call with deep-equal arguments.
- Real desktop app: lock an app drafted on the fake server, open it from a chat on another agent, read, approve a write, and see the fake server log the exact arguments.
