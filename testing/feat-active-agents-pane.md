# feat/active-agents-pane — test contract

The dock's Agents tab lists the agents a chat has running now, and nothing else.

## Pane — `src/components/TasksPane.test.tsx`

- [x] Lists only workers under this chat (any depth) whose status is working or waiting. Finished, failed, cancelled, and other chats' workers are not listed, even though the chat's snapshot carries the whole workspace.
- [x] A queued request is listed only if its `queueStatus` is `queued` and it belongs to this chat.
- [x] Nothing running reads "No agents running".
- [x] A row expands in place to the worker's own transcript (`WorkerDetail`, embedded: a region, never a dialog) and collapses again.
- [x] A pinned worker is listed first and open, and stays listed after it finishes, until unpinned. A finished row offers no Stop.
- [x] Stop and the shells row route through the host.

## Pins — `src/pinnedAgents.test.ts`

- [x] Pins round-trip through `localStorage["bridge.agents.pinned"]`; missing or malformed values read as none.

## A chat stays busy while its agents run — `sidebarChats.test.ts`, `BridgeSidebar.test.tsx`, `ComposerPill.test.tsx`

- [x] Live descendant sessions count toward the chat at the top of the tree; an idle chat with live agents is Active and reads `N agents working` on a `bg-info` dot.
- [x] The composer keeps Stop beside a plain Send while agents run under an idle turn; Stop ends the turn if one is live and every live agent under the chat.

## App — `src/App.test.tsx`

- [x] The tab is labelled Agents and its count is the pane's running count.
- [x] The sixth chord opens it with the running worker and the queued request, and without the finished one.
