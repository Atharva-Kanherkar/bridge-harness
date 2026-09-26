# feat/agents-pane — test contract

Locked before implementation. One workstream: the dock's Tasks pane grows into
an **Agents** pane that is the live, expandable, pinnable home for Bridge
workers *and* harness-native subagents (Claude `Task`, Codex collab agents,
OpenCode `task`). The orchestrator chat keeps one pointer line per delegation,
per subagent and per ask. A pinned-agents tray follows you across dock panes
and chats. The Claude normalizer attributes `Task` subagent events the way the
OpenCode normalizer already does.

Not in scope: Mission Control, Agent Fleet, any new status/tone/lifecycle,
remote and SSH hosts, notifications. No protocol change.

## Shape of the thing

`src/components/agentsModel.ts` is the single pure projection. Input:

```
agentsModel({ sessions, forestsBySession, transcriptsBySession, events, now })
```

Output: `AgentRun[]`, one per root session, each

```
AgentRun    { rootSessionId, title, harness, startedAt, agents: AgentNode[], census }
AgentNode   { id, source: "worker" | "subagent", harness, name, parentId, depth,
              status: WorkerStatus, liveLine, startedAt, endedAt, children,
              sessionId, model, branch, scope, objective, failureCode, ask }
```

`src/agentsPaneSettings.ts` persists pin, expansion, acknowledgement and scope
under `localStorage["bridge.agents-pane"]`, **not** keyed by chat.
`src/components/AgentsPane.tsx` renders rows, the expanded body, the drill-in
view and the pinned tray. `SessionDock` grows an optional `tray` slot rendered
below the pane container on every pane except Agents. The dock pane keeps the id
`tasks` (persisted-state compatibility), the label `Agents` and the `Network`
icon. `WorkerDetail`'s internals back the drill-in view; the `absolute inset-0`
overlay over the chat section is gone. `AgentConversation` renders
`DelegationPointer` / `SubagentPointer` / `NeedsYouPointer` instead of the
inline `WorkerPanel` card. `src-tauri/bridge-core/src/agent.rs` stamps
`data.subagent` on Claude messages carrying `parent_tool_use_id`.

## 1. The projection — `src/components/agentsModel.test.ts`

| # | Behaviour | Assertion |
|---|---|---|
| 1.1 | Workers and subagents merge into one tree | an orchestrator forest with three runtimes plus a transcript carrying a `data.subagent` group yields one `AgentRun` with four nodes, `source` `worker` for the runtimes and `subagent` for the group |
| 1.2 | A subagent nests under the worker whose transcript it arrived in | the subagent's `parentId` is that worker's id and its `depth` is 1 |
| 1.3 | A subagent of the orchestrator sits at depth 0 | a `data.subagent` group in the root session's transcript has `parentId` undefined |
| 1.4 | Census counts | `census` reports running/needs-you/done/queued totals across the run and is zeroed for an empty run |
| 1.5 | A missing runtime is quiet | a worker session with no runtime record yields `status.tone` `working` with label `STARTING`, never `attention` |
| 1.6 | Machine fences never become a live line | an event whose text is ```` ```bridge-worker-result ```` produces `liveLine` undefined |
| 1.7 | Finished rows persist | a runtime reported `completed` still yields a node, with `endedAt` set |
| 1.8 | Acknowledgement dims, never hides | an acknowledged failure is still in `agents` |
| 1.9 | Scope filters runs | `scope: "this-chat"` returns only the requested root; `all-chats` returns every root with a forest |
| 1.10 | A subagent's live line comes from its own rows | the newest attributed tool row becomes the live line, and the parent's own rows do not leak into it |
| 1.11 | The Codex collab row shows status without a live step | a `SubagentFacet` tool call with `status: "running"` and no attributed rows yields status `working` and `liveLine` undefined |
| 1.12 | A worker's live line is one legible step | `workerPanelModel` with a feed limit of 1 supplies `liveLine`; the expanded feed is 5 |
| 1.13 | The ask is read off the child approval | a pending `delegation.blocked` approval for a worker becomes that node's `ask`, carrying command, cwd, write scope and the approval's event id |
| 1.14 | A failure shows a stable code | a failed runtime's `failureClass` becomes `failureCode`, and raw provider error text is not copied into it |

## 2. Settings persistence — `src/agentsPaneSettings.test.ts`

| # | Behaviour | Assertion |
|---|---|---|
| 2.1 | Round trip | pinned, expanded and acknowledged ids and the scope survive a write/read round trip |
| 2.2 | Not keyed by chat | writing for chat A and reading for chat B returns the same record |
| 2.3 | Corrupt input degrades to defaults | malformed JSON, a non-object, or a wrong-typed field yields the default record, never a throw |
| 2.4 | Unknown ids survive | a pinned id that is no longer in the model is still stored, so unpinning is the only way to drop it |
| 2.5 | The store is not chat-scoped | the storage key is exactly `bridge.agents-pane` |

## 3. The pane — `src/components/AgentsPane.test.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 3.1 | A collapsed row is one live line | the row renders chevron, dot, `HarnessMark`, name, the `worker`/`subagent` tag, the clock and one step line; the expanded body is absent |
| 3.2 | Clicking a row expands it in place | the objective, model, branch, scope chips, the last five steps, the `N files · +A −B · N tool calls · ctx N% · $cost` footer and Transcript/Steer/Stop/Pin render; children are visible while expanded |
| 3.3 | Nesting is an indent plus a hairline | a child row carries `ml-[15px] border-l pl-2` |
| 3.4 | A NEEDS YOU row answers inline | Approve calls the injected resolver with the approval's event id and never calls `onOpenSession` |
| 3.5 | A failed resolve stays NEEDS YOU | a rejecting resolver leaves the row in NEEDS YOU and renders the error inline |
| 3.6 | A failed row shows the stable code | `sandbox_denied`-style code in mono destructive plus the last legible step; Retry and Open transcript render; the raw provider text does not |
| 3.7 | Finished rows dim and stay | a done row is still rendered after the status flips |
| 3.8 | Pinning puts the row in the tray | Pin moves the id into settings and the row carries the pinned mark |
| 3.9 | The tray renders on a non-Agents pane | given pane `github`, the tray lists the pinned agent with its chat title and live step |
| 3.10 | The tray's ask action is inline | the tray's Approve calls the same resolver |
| 3.11 | The tray is collapsible | the header count renders and hiding it removes the lines |
| 3.12 | Drill-in replaces the overlay | Transcript renders the breadcrumb, the four counters, the Activity/Files/Prompt/Result tabs and a steer box, and no `absolute inset-0 z-30` overlay exists |
| 3.13 | Queued rows and shells stay at the bottom | the queue entries and the running-shell row render after the agent rows |
| 3.14 | The footer census renders | the footer reports running, queued and needs-you |

## 4. App wiring — `src/App.test.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 4.1 | The pane is labelled Agents | the descriptor is `{ id: "tasks", label: "Agents", icon: Network }` |
| 4.2 | The pane is not remounted per chat | the mount carries no `key={session.id}` |
| 4.3 | The first agent opens the dock once | with a forest that has workers, the dock opens on `tasks`; a second render does not re-open a dock the user closed |
| 4.4 | The attention dot rides the descriptor | a pending, unacknowledged ask sets `alert` on the pane and on the toolbar |
| 4.5 | Acknowledgement persists across reload | the acknowledged set is read from `agentsPaneSettings`, not from component state |
| 4.6 | The chat pointer focuses the row | clicking a delegation pointer dispatches `open-pane tasks` and focuses the child id |
| 4.7 | The overlay no longer mounts | `expandedWorkerId` no longer renders `WorkerDetail` over the chat section |
| 4.8 | All-chats scope uses the digest | `sessionForestDigest` is polled and `sessionForest` is fetched only on a changed digest |

## 5. Transcript pointer lines — `src/components/AgentConversation.test.tsx`

| # | Behaviour | Assertion |
|---|---|---|
| 5.1 | A delegation is one pointer line | the line reads `Delegated N workers`, carries the harness marks and the mono census and ends with `Agents ›`; no inline worker card renders |
| 5.2 | Clicking it focuses the row | the click calls the focus callback with the child session id |
| 5.3 | A subagent call is one line | agent type, description, live dot and clock render |
| 5.4 | A child approval is one line with a warning edge | the line carries `border-l-2 border-warning` and reads `Answer in Agents ›` |
| 5.5 | The detail of a subagent is not in the chat | `SubagentBlock`'s prompt/result bands do not render inline |

## 6. Claude attribution — `src-tauri/bridge-core/src/agent.rs`

| # | Behaviour | Assertion |
|---|---|---|
| 6.1 | A parented message is attributed | an assistant message carrying `parent_tool_use_id` gets `data.subagent = { sessionId, agent, title }` on every event it produces, with `sessionId` equal to that `tool_use_id` |
| 6.2 | `agent` and `title` come from the Task call | `agent` is the Task call's `subagent_type` and `title` its `description` |
| 6.3 | A top-level message is unchanged | a message without `parent_tool_use_id` produces no `subagent` key |
| 6.4 | OpenCode is untouched | the existing OpenCode subagent tests still pass |

## 7. Design system

| # | Behaviour | Assertion |
|---|---|---|
| 7.1 | Tokens only | `src/designSystem.test.ts` passes untouched: no palette utility, no white/black alpha, no raw hex, no blur on a resting surface |
| 7.2 | No new tone | the dot classes come from the existing `TONE_DOT` map in `TasksPane.tsx` |

## Smoke

- `bun run build` and `bun run test` are green.
- `bunx vitest run src/designSystem.test.ts` passes.
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core agent::` passes.
- Nothing under `src/protocol/**` is edited.
