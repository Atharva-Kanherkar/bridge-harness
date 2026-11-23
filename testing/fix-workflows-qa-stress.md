# fix/workflows-qa-stress: Test Contract

Locked before implementation. A QA pass over Workflows Phase 1 (#845, #962)
against real Claude through a scratch `bridged` found the defects below. Each
row names the live reproduction; each fix lands with a test that fails without
it. Phase 2 is out of scope.

## Defects and the behaviour they must end in

| # | Live reproduction | Required behaviour |
| --- | --- | --- |
| D1 | A no-project workflow granted `claude.ai Notion` read, Run now: `failed / not_done` in 15 s, "the claude.ai Notion connector needs OAuth authorization, and no Notion search tool is available". The SDK init frame shows `claude.ai Notion=needs-auth`, 0 tools. | A granted `claude.ai …` connector is loaded by Claude Code itself (signed in), never as an explicit copy. The run's first turn waits until each granted connector has left `pending` (bounded at 30 s). The same run succeeds with the real search result. |
| D2 | In a workflow run, `ListAgents`, `CronCreate`, `CronDelete`, `SendMessage` and `RemoteTrigger` (`list`) executed with `canUseTool` never asked. `ListAgents` listed the person's other live sessions. | A connector workflow run has no built-in tools at all (`tools: []`): its only tools are connector tools, every one decided by the workflow gate. Ungranted configured servers are denied (`deniedMcpServers`), and any ungranted server that still appears is switched off before the first turn. A worktree run gets an allow-list of coding built-ins; the cross-session, remote, scheduling and worktree-switching built-ins are absent, and the PreToolUse hook denies them too. |
| D3 | Stop pressed while the run was `running` but its provider was still starting: the run read `cancelled`, the session started 7 s later and ran 25 s more. | A runner checks that its run is still active (and still its lease) before starting the provider and on every poll. Stop, delete and reclaim end the session within one poll (500 ms), not at the next 30 s heartbeat. |
| D4 | Delete mid-run: the session kept working about 30 s in a folder delete had already removed; its `workflow` session row stayed. | Delete cancels the workflow's active run and stops its session before removing rows and the folder. |
| D5 | SIGKILL of the daemon mid-run, restart: after 35 s the run still read `running`; Run now returned it with `created: false`. | At boot, before its first tick, the maintenance thread settles every run leased by a previous owner as `failed / interrupted` with its recorded usage. A parked quota run (no lease) stays parked; a quota run caught mid-recheck has its stale lease released. Run now works at once. This amends F11 and E3: a data directory has exactly one owner (`bridge_core::ownership`), so a lease present at boot cannot belong to a live worker. The 15-minute lease still covers a stuck worker inside one process. |
| D6 | `workflows/workflow_options` reports `worktreeAllowed: true` for Codex, Cursor, Grok, OpenCode and Pi; saving such a definition is refused. | `worktreeAllowed` is the same gate validation applies. The editor shows that gate's reason for a disabled harness on a worktree job. |
| D7 | The run view loads `replaySessionEvents(sessionId, 0, 500)`: a run past 500 durable events never shows its pending approval card or its last messages. | The run view pages through every event. |
| D8 | Found in review, not live (no camelCase connector is signed in here): the sidecar splits `mcp__<server>__<tool>` only when the tool is all lowercase, so on a granted Atlassian server `createJiraIssue` and `getJiraIssue` are denied as "not a connector this workflow may use", and Always allow on such a card is refused as "not an exact tool identity". | A tool with any non-empty name identifies its server. Its words are never read as a verb unless it is all lowercase, so it is a write: it asks on a `write: "ask"` grant and is refused by name otherwise. Always allow records it. Briefing scoped reads and approved actions are unchanged. |

## Unit Tests

- Sidecar `test/workflow.mjs`:
  - a connector workflow's options carry `tools: []`, `strictMcpConfig: false`, no explicit `claude.ai …` copy, and deny every configured server it was not granted (D1, D2);
  - a definition granting only non-`claude.ai` servers keeps `strictMcpConfig: true` and its explicit copies;
  - the connector wait resolves once every granted server has left `pending`, switches off each ungranted server it sees, keeps polling while a granted server is missing or `pending`, survives a throwing status call, and gives up at its deadline (D1, D2);
  - a worktree workflow's options carry the coding allow-list, none of the escape built-ins, and the hook denies `SendMessage`, `RemoteTrigger`, `PushNotification`, `Workflow`, `CronCreate`, `EnterWorktree` (D2);
  - ordinary chat options are unchanged (no `tools`, no `deniedMcpServers` added).
- `workflow_run` (Rust): a run cancelled after `running` but before its provider is waited on ends with `LeaseLost` on the first poll, not after the heartbeat interval; a deleted run does too (D3, D4).
- `workflows`/`api` (Rust): deleting a workflow with an active run cancels that run before deletion and returns its session for stopping (D4).
- `workflow_trigger` (Rust): boot settlement fails `queued`, `running` and `waiting_approval` runs that hold a lease with `interrupted`, sums their usage, releases a leased `waiting_quota` run back to parked, leaves an unleased parked run and terminal runs alone, and publishes each change once; a manual claim then succeeds (D5).
- `api::workflow_options` (Rust): `worktreeAllowed` is false with the gate's reason for every adapter `adapter_may_brief` refuses (D6).
- Sidecar and `workflows` (Rust): on a granted `write: "ask"` server a camelCase tool asks; on a read-only grant it is refused by name; Always allow validates it; `tool_class` calls it a write; a briefing still never reads it as a scoped read (D8).
- Frontend: `harnessRefusal` uses the gate's reason for a worktree job (D6); `WorkflowRunView` shows the pending card of a run whose approval is event 600 (D7).

## Integration / Functional Tests

- `cargo test -p bridged --test workflows`: E1, E2, E4 unchanged; E3 updated per D5 (the restarted owner settles the run as `interrupted` without a fake clock; the executed write is still not repeated).
- `cargo test -p bridge-core workflow` and `node --test sidecar/claude-agent/test/*.mjs`.

## Smoke Tests

- Scratch `bridged` (separate data dir, `--health-addr none`, started under `setsid`), real Claude, real HOME:
  - D1: the Notion read workflow ends `succeeded` with the real count.
  - D2: a probe through the real sidecar options lists no built-ins in the init frame.
  - D3, D4, D5: the three races re-run: the session is gone within a second of Stop or delete; after SIGKILL and restart the run reads `failed / interrupted` and Run now returns `created: true`.
  - Write approval on a real connector, using a call that fails harmlessly (an update on a page id that does not exist): the run waits as `waiting_approval`, Allow once executes the call once, and the effect is recorded.
  - Dry run: the same write is recorded as `previewed`, never sent.
  - A `*/15` schedule fires once per slot.

## E2E Tests

- The release-app journeys M1 to M4 of `feat-workflows-phase-1.md` stay as recorded in #962. This PR adds browser screenshots of the editor's worktree refusal (D6) and the run view with a late pending card (D7), light and dark.

## Manual / cURL Tests

N/A beyond the scratch-daemon smoke above: no HTTP endpoint changes.

```sh
node --test sidecar/claude-agent/test/workflow.mjs
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core workflow
cargo test --manifest-path src-tauri/Cargo.toml -p bridged --test workflows
bun run build
bun run test
```
