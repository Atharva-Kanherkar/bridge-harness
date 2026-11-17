# Personal app drafts

A top-level chat on Claude Code, Codex, OpenCode or Cursor can build a small
app around one MCP server with `bridge.apps_draft`. The transcript card runs live reads and previews writes
as “would send” results. Lock saves the reviewed content hash; Revise fills the
composer. A revision that adds access needs a second native confirmation.
Saved-app launch/routing and actual writes come in the next slice.

The tool takes `name`, `intent`, `server`, bare `reads`/`writes`, `view_html`,
optional `sample_args` and optional `app_id` for a revision. Use real tools from
the existing chat's MCP inventory. A view uses MCP Apps JSON-RPC over
`parent.postMessage`: `ui/initialize`, `ui/notifications/initialized`,
`ui/notifications/tool-input` and `tools/call` with a bare tool name. It should
report `ui/notifications/size-changed` after layout/content changes. Use inline
scripts/styles and data assets; network and external links are unavailable.

## One router for every agent

Each app is bound to a connection: `{harness, account, server}`. The chat that
drafts or previews it can be any of the four agents; the connection must be on
an agent Bridge can call directly (Claude Code or Codex today). Drafting looks
for the server on the chat's own agent and account first, then on the person's
own Claude Code and Codex logins, and refuses with "connect it in Claude Code
or Codex" when none has it. OpenCode and Cursor connections come in a later
slice through a gated path.

All MCP work goes through `apps::router`, which checks the allowlist, the
read vocabulary and the chat's access mode before anything is sent. It uses
the chat's own live runtime when the chat is that connection, otherwise a warm
connection runtime (`apps::runtimes`): a hidden process for that exact
connection with no Bridge tools and no model turn, stopped after 10 idle
minutes. Claude's is a briefing whose only authority is app control on the one
server; Codex's is `codex app-server` with an ephemeral thread, using
`mcpServerStatus/list` and `mcpServer/tool/call`.

The sidecar asks the existing Claude query for inventory and calls safe reads
without a model turn. Only sanitized tool names and safety annotations cross
the inventory channel. Replies are chat-bound and consumed before transcript
handling. Both core and sidecar enforce read classification; writes stop in core
at a stub. The sidecar also applies the chat's own access mode: a preview reads a
server only where that mode would let the model read it, so Shared and Isolated
chats, and offline read-only chats, refuse live reads. Another tool, chat, expired draft or changed hash refuses. Drafts last
24 hours. A locked card reloads its original version and remains a preview.
If a chat's runtime has stopped, resume the chat to reconnect its MCP servers.

Personal HTML runs in an opaque-origin sandbox inside a second trusted sandbox.
The outer CSP also governs inner-frame navigation: a view cannot navigate itself
to send read data out. The relay forwards only between the mounted inner frame
and its parent. Personal frames have no message-to-composer or open-link
capability. Live tools and native changes are disabled in history.

CSP does not cover WebRTC in any engine, so a view could still send read data
out through STUN/TURN traffic. The Mac app turns WebRTC off in its own webview
(`src-tauri/src/preview_isolation.rs`, WKPreferences `_peerConnectionEnabled`)
and reads the setting back. The card mounts a view only after that check, and
allows live reads only when it succeeded. A browser tab (web mode) and other
platforms cannot switch WebRTC off, so they refuse live reads; writes still
preview there.

Migration 82 adds draft/chat bindings; optional sample arguments are omitted
from old manifests so existing hashes remain unchanged. Protocol 1.39 adds
`apps/get_personal_app_draft` and `apps/preview_personal_app_tool`.

## Opening and using a locked app

A top-level chat on any of the four agents opens a locked app with
`bridge.apps_open({app_id, prefill?})`. The tool's description lists the
person's locked apps (name, id and intent, at most 20) as of the chat's launch.
Opening binds the chat to the app's latest locked version and content hash
(migration 83) after checking the connection has the server connected.

The card shows the stored, hash-checked view with the prefill as its input.
Every call from the view comes back to core, which rechecks the chat, the open
and the hash:

- **Allowlisted read:** goes through the router: access mode and current annotations.
- **Allowlisted write:** never runs on its first call.
  - Core canonicalizes the arguments once and returns them with a single-use token (600 s, one pending per open).
  - The card shows that exact JSON in the approval sheet.
  - Send claims the token, and core sends the arguments it stored, so the view cannot change them after approval.
  - Claude connections send through a one-shot exact-action launch, whose sidecar and SDK gate each check deep equality. Codex connections use `mcpServer/tool/call`.
- **Anything else:** refused.

Writes always ask, in every access mode, as Outgoing does. Each call leaves an
`events` row (`personal_app.call.read|write|refused`) without arguments or
results. Protocol 1.41 adds `apps/get_personal_app_open` and
`apps/call_personal_app_tool`.

## Verification

Focused tests are listed in `testing/feat-personal-app-drafts.md`. The real-browser
smoke uses a fresh unsigned profile, demo data and the actual frame builder:

```sh
bun run dev --port 1462
# In another terminal, if Playwright is not already installed:
npm install --prefix /tmp/bridge-preview-smoke playwright
npx --prefix /tmp/bridge-preview-smoke playwright install chromium webkit
BRIDGE_PLAYWRIGHT_MODULE=/tmp/bridge-preview-smoke/node_modules/playwright/index.mjs node scripts/smoke-personal-apps.mjs
BRIDGE_PREVIEW_BROWSER=webkit BRIDGE_PLAYWRIGHT_MODULE=/tmp/bridge-preview-smoke/node_modules/playwright/index.mjs node scripts/smoke-personal-apps.mjs
```

This checks samples, read replies, dry-run writes, native Lock/Revise, both
themes, and actual CSP blocking of network/navigation attempts. A browser
cannot exercise the WebRTC switch, which needs the Mac app's webview. Screenshots
are written under `docs/design/personal-apps/`.

With a real Claude account, ask a chat to build an app for a connected server,
check its access list, run a read, preview a write, Lock and reload. The original
locked preview should return; a tool outside its allowlist should refuse.
