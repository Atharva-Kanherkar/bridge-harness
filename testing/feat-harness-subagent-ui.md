# feat/harness-subagent-ui — Test Contract

## Functional Behavior
- Agents pane lists active harness_subagent child sessions without worker runtimes alongside existing workers, scoped to the selected chat.
- Child rows use existing agent chat/pin/Stop controls and show a subagent label, current step and honest permission description: Started by <harness> in this chat. Runs with this chat’s permissions. Not a Bridge worker.
- Selecting a child shows its own forest and approvals. No child composer or steering is offered for a processless session.
- Parent transcript folds each child lifecycle into one quiet pointer with Show in Agents. Provider launch placeholders never render as results. Original Agent tool cards are replaced by the attributed pointer when available.
- Finished rows remain readable if selected or pinned; running rows remain until provider completion. Direct chats auto-open newly observed child sessions through the same spawn watcher as workers.
- Existing Bridge workers, grouping and transcript behavior remain unchanged. Tailwind v4 utilities and existing tokens only.

## Unit Tests
- TasksPane.activeAgentRows includes runtime-free subagents, excludes another chat, retains pinned completed children, and preserves ordinary workers.
- TasksPane opens child AgentChat, labels the provenance, shows only the child's messages/approvals, and routes Stop with the child Bridge session id.
- Transcript folding replaces matching Agent tools with a single lifecycle pointer; completion updates that pointer without hiding unrelated tools or child chat content.
- AgentConversation renders Show in Agents and no async placeholder; workerStatus accepts processless children while preserving unavailable-runtime errors for workers.

## Integration / Functional Tests
- Feed backend lifecycle pointer events and child-session snapshots into the actual TasksPane and AgentConversation; navigate pointer → child chat → all agents and verify Stop addresses only the selected child.
- Existing worker, approval, grouping and spawn watcher regression tests pass.

## Smoke Tests
- bun run build and bun run test pass before PR creation.

## E2E Tests
- Fresh local browser renders the actual components with a deterministic backend-shaped fixture: parent pointer, three child rows, one waiting approval, child prose/tool calls; open child, return to list, stop a child.
- Capture screenshots in light and dark themes and embed them in the UI PR description. Screenshots are fixtures, explicitly identified as such.

## Manual / cURL Tests
- Open the local fixture route; select Show in Agents, pin and stop the child; verify it remains readable and sibling stays working.
- Main Direct chat uses activeAgentRows, so newly arriving child sessions trigger the existing spawn watcher; verify with current unit coverage and inspect App wiring.
