# feat/personal-app-router — Test Contract

Issue #1026, slice 4a-1. Plan: "Plan v2: personal apps on every agent (router), slice 4" in Bridge Notes.

## Functional Behavior

- A personal app is bound to one connection: `{harness, account, server}`. The manifest gains an optional `account` (omitted when it is the default login), so existing manifests hash the same. `harness` names the connection's agent, not the chat's.
- Connection harnesses with direct MCP calls: `claude`, `codex`. Each adapter declares a capability per operation (`inventory`, `read`, `write`): `direct`, `gatedCourier`, `partial` or `unsupported`. In this slice `opencode` and `cursor` are `partial`/`unsupported` for connections, and Grok and Pi are `unsupported`. Every refusal names the agent and what to do.
- All MCP work for apps goes through one core router. The router checks, before any adapter runs:
  - that the tool is on the app's allowlist (reads only in this slice), with Bridge's read vocabulary;
  - that the stored content hash still matches;
  - that the requesting chat's access mode allows connector reads: Full yes; Shared and Isolated no; ReadOnly only with network authority.
  Adapters never decide authority.
- Path selection:
  - Use the chat's own live runtime when its harness and account are the connection's.
  - Otherwise use a warm connection runtime for `{harness, account}`. It is a hidden, turnless process with no Bridge tools, reused across calls, and stopped after 10 idle minutes or at shutdown. Concurrent first calls share one launch.
  - A Claude runtime is a sidecar launch whose only authority is app control on the one server: reads and inventory. A Codex runtime is `codex app-server` with an ephemeral thread and no Bridge MCP server.
- Codex adapter:
  - Inventory is `mcpServerStatus/list {serverName, threadId, detail: "toolsAndAuthOnly"}`, normalized to `{name, status, tools[{name, readOnly, destructive}]}` from MCP annotations (`readOnlyHint`, `destructiveHint`).
  - A read is `mcpServer/tool/call {threadId, server, tool, arguments}`.
  - Requests use string ids namespaced to Bridge. Their responses are consumed before transcript handling, never shown to the model, and bounded as for Claude. Errors never relay server text.
- Drafting works from top-level chats on Claude, Codex, OpenCode and Cursor. The connection is chosen in this order:
  1. The chat's own `{harness, account}`, when it is direct-capable and has the server connected.
  2. Otherwise the default Claude login, then the default Codex login.
  Exactly one connected match is used. If no direct-capable agent has the server connected, the draft is refused with a message to connect it in Claude Code or Codex.
- Previews read through the router on the draft's connection. Writes remain dry-run stubs.
- The card shows the connection ("claude.ai Slack · via Codex"). A slower or unavailable path is stated in plain words.
- Cursor chats receive Bridge's MCP server (ACP `session/new` `mcpServers`), so `apps_draft` and Bridge's other chat tools are reachable from Cursor as from the other agents.
- No real writes, no `apps_open` and no approval sheet in this slice (4a-2).

## Unit Tests

- Manifest: a legacy manifest without `account` keeps its hash. `account` bounds. The harness must be a connection harness.
- Router:
  - Allowlist, read vocabulary and annotations refuse.
  - Each access mode allows or refuses as above.
  - A hash mismatch refuses.
  - Live and warm path selection: live only for the same harness and account; never another account.
  - Capability refusals name the agent.
- Codex adapter:
  - Request frames carry the exact server, tool and arguments.
  - Inventory normalization from MCP annotations.
  - Response correlation by namespaced id; foreign or stale ids are ignored.
  - Error text is not relayed.
  - Size limits.
- Warm runtime pool:
  - Concurrent callers share one launch.
  - Idle expiry stops the process.
  - Accounts are kept apart.
  - A failed launch is not cached.
- Claude app runtime: the sidecar enables app control only for the policy's server. It refuses inventory and reads for any other server, and refuses writes.
- Connection choice for drafts: chat-first; the Claude-then-Codex fallback; no match refuses; ambiguous wire names refuse.

## Integration / Functional Tests

- Draft → preview read → lock from a Codex chat whose connection is Codex, and from an OpenCode chat whose connection is Claude, against fake runtimes. Locked previews still reload and stay dry-run.
- The authenticated tool route lists `apps_draft` for top-level Claude, Codex, OpenCode and Cursor chats, and not for workers.
- The Cursor ACP session request includes Bridge's MCP server.
- Wire, method and daemon/Tauri dispatch stay consistent with the regenerated protocol.

## Smoke Tests

- Real desktop app (isolated identifier and data dir): a fake stdio MCP server configured in Codex. A Codex chat drafts an app; the preview's read returns the fake server's data through `mcpServer/tool/call`; the write shows "would send".
- The same app previewed from a Claude chat reads through a warm Codex runtime.

## E2E Tests

- Live Codex app-server against the fake MCP server: inventory annotations, a direct read, and no model turn.

## Manual Tests

- With real accounts: a Slack app drafted in Claude previews from Codex and Cursor chats; a server connected only in OpenCode is refused with the "connect it in Claude Code or Codex" message.
