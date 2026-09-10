# feat/mission-control-live-grid — Test Contract

Two screens, two jobs. **Agent Fleet** is the terminal split workspace for agent CLIs. **Mission Control** is a live grid of every active chat session, each tile a real, working conversation with its own composer, just resized.

## Functional Behavior

### Mission Control (`src/components/MissionControl.tsx`)
- Shows every *active* session (status `working`, `waiting`, `starting`, `resuming`, `checkpointing`, or a worker whose runtime is running) as a tile. Idle/completed sessions are hidden by default behind a "Show all" toggle.
- Each tile renders the real `AgentConversation` for that session (live `events` filtered by `sessionId`, plus the session's forest loaded via `bridgeApi.sessionForest`) and a real composer that submits through `bridgeApi.submitInput(sessionId, text)`. Approvals and questions resolve through `bridgeApi.resolveApproval` / `bridgeApi.resolveQuestion` for that tile's session, never the globally selected one.
- Tiles are laid out with the split-tree model from `src/terminal/layout.ts` (`PaneNode`), so they can be resized with drag separators and rearranged by dragging a tile header onto another tile's edge. New active sessions auto-insert into the tree; sessions that stop are removed. Layout persists in `localStorage` under `bridge.mission-control.layout`.
- A tile header shows title, harness, status tone (reuse `workerStatus`/tone vocabulary), and actions: focus (opens the session in the single view via `onFocusSession`), maximize/restore, and stop for workers (`onStopWorker`).
- The tile whose session is `activeSessionId` carries a subtle ring.
- Empty state explains that active chats appear here automatically and offers a button to open Projects/New chat via `onFocusSession` alternatives (no new App props).
- Achromatic chrome; only status ink carries hue. Tailwind v4 utilities only.

### Agent Fleet (`src/components/TerminalWorkspace.tsx`, `src/components/AgentFleet.tsx`)
- "New Agent → <harness>" and "New Terminal" open the new pane **in the current tab as a split of the focused pane**, not as a new tab. Direction is chosen automatically so the grid stays balanced: split horizontally when the focused pane is wider than tall, vertically otherwise (measure the pane element; fall back to alternating by depth when unmeasurable).
- A secondary "New tab" action (menu item or ⌘T-style command) still exists for people who want a separate tab; `new-tab` shortcut keeps working.
- Dragging a pane header onto another pane's edge moves it there (already present) and additionally onto the **empty tab-panel background** appends it as a new split of the last leaf. Drag affordance is discoverable (grip icon + cursor).
- Header dedupe: the workspace title no longer repeats the branch that is already shown on the right. Tab labels use the agent display name ("Claude Code", "Codex") instead of the raw id.

## Unit Tests
- `src/components/MissionControl.test.tsx`: active filter and Show-all toggle; a tile renders the conversation and composer; submit calls `bridgeApi.submitInput` with that tile's `sessionId`; approval resolve targets the tile's session; new session auto-inserts; removed session drops out; layout persists and restores; focus button calls `onFocusSession`.
- `src/terminal/layout.test.ts`: `autoSplitDirection` helper (or equivalent) and "append to last leaf" insertion.
- `src/components/TerminalWorkspace.test.tsx`: New Agent splits into the current tab (tab count unchanged, leaf count +1); New tab still creates a tab; drop on empty panel background appends; tab label shows agent display name.
- Existing `AgentFleet.test.tsx`, `BridgeSidebar*.test.tsx`, `navigationHistory.test.ts`, `workWiring.test.ts`, `App.test.tsx` stay green.

## Integration / Functional Tests
- `bunx tsc -b` clean. `bunx vitest run` clean for the files above plus `src/App.test.tsx`.

## Smoke Tests
- Preview mode (`vite --port 1468`, accept model setup, dark class): sidebar shows Agent Fleet then Mission Control; Mission Control shows the mock working orchestrator as a live tile with a composer; Agent Fleet "New Agent → Claude Code" splits the current tab.

## E2E Tests
- N/A — native desktop automation is not configured; preview-mode smoke plus component tests cover the boundaries.
