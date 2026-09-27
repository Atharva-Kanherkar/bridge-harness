# feat/agents-pane-auto-open — test contract

When an orchestrator starts an agent, the dock opens on the Agents pane with that agent's chat, drawn the way every Bridge chat is. The orchestrator's transcript no longer carries a live worker card.

## Spawn detection — `src/agentSpawns.test.ts`

- [x] The first reading for a chat seeds what it has running and reports nothing, so opening a chat with agents already running does not pop the dock.
- [x] A live agent id not seen before for this chat is reported as spawned; the newest id is the one to focus.
- [x] Switching chats reseeds instead of reporting the other chat's agents as spawned.
- [x] An agent that finishes and is gone is not reported again.

## Pane — `src/components/TasksPane.test.tsx`

- [x] The list still holds only this chat's working or waiting agents, queued requests for this chat, and pinned agents (pinned first, kept after they finish).
- [x] A row opens that agent's chat in the pane (the list gives way to it), and Back returns to the list.
- [x] A `focus` request opens the named agent's chat, and a repeat request with a new nonce opens it again after Back.
- [x] An open agent stays open after it finishes, even when unpinned and no longer listed.
- [x] Pin, Stop, Open session, and the shells row still route through the host. A finished agent offers no Stop.

## Agent chat — `src/components/AgentChat.test.tsx`

- [x] Renders the worker's forest entries and live frames through `AgentConversation` (message bubbles and tool rows, not the old mono feed), with no dialog semantics.
- [x] Offers the steer box while the worker is live, the finished notice once it has reported, and no steer box while it is checkpointing.
- [x] Once the worker has reported, its chat ends on the typed result (status, summary, files, checks), including when its last message was only the `bridge-worker-result` fence the transcript strips. An unreported `lastResult` is not shown.

## Orchestrator transcript — `src/components/AgentConversation.test.tsx`

- [x] A spawn renders one quiet "Delegated" row, never a worker card (no status label, feed, clock, or Stop).
- [x] That row offers "Show in Agents" when the host can open the dock, and calls it with the child session id.
- [x] A successful result is the quiet "Subagent finished" row; a classified failure keeps its alert and Retry.

## App — `src/App.test.tsx`

- [x] The Agents tab still opens on the sixth chord with the running worker and queued request; a row opens the worker's chat and Open session makes it the selected conversation.
