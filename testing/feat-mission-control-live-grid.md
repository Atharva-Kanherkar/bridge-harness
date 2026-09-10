# feat/mission-control-live-grid — Test Contract

Two screens, two jobs. **Agent Fleet** is the terminal split workspace for agent CLIs. **Mission Control** is a live grid of every active chat session, each tile a real, working conversation with its own composer, just resized.

## Functional Behavior

### Mission Control (`src/components/MissionControl.tsx`)
- Shows every *active* session (status `working`, `waiting`, `starting`, `resuming`, `checkpointing`, or a non-null active turn) as a tile. Cached worker runtime snapshots do not keep finished chats visible. There is no "Show all" toggle.
- Drag any sidebar chat into the empty canvas or onto a tile's left/right/top/bottom edge to pin it, even while idle. Only explicitly dragged chats stay after completion; other idle history stays hidden. Pins persist with the layout. Remove an idle pin with its close button, or unpin a working chat so it leaves when finished. Removing a pin never archives, stops, or deletes the chat.
- Each tile renders the real `AgentConversation` for that session (live `events` filtered by `sessionId`, plus the session's forest loaded via `bridgeApi.sessionForest`) and a real composer that submits through `bridgeApi.submitInput(sessionId, text)`. Approvals and questions resolve through `bridgeApi.resolveApproval` / `bridgeApi.resolveQuestion` for that tile's session, never the globally selected one.
- Tiles are laid out with the split-tree model from `src/terminal/layout.ts` (`PaneNode`), so they can be resized with drag separators and rearranged by dragging a tile header onto another tile's edge, with a labeled edge preview. Drafts stay with their session when tiles move. New active sessions auto-insert into the tree; unpinned sessions that stop are removed. Tiles have a 420 × 360 minimum size, with scrolling when necessary. Layout persists in `localStorage` under `bridge.mission-control.layout`.
- A tile header shows title, harness, status tone (reuse `workerStatus`/tone vocabulary), and actions: focus (opens the session in the single view via `onFocusSession`), maximize/restore, and stop for workers (`onStopWorker`).
- The tile whose session is `activeSessionId` carries a subtle ring.
- Empty state explains automatic active chats and dragging sidebar chats into the grid.
- Achromatic chrome; only status ink carries hue. Tailwind v4 utilities only.

### Agent Fleet (`src/components/TerminalWorkspace.tsx`, `src/components/AgentFleet.tsx`)
- "New Agent → <harness>" and "New Terminal" open the new pane **in the current tab as a split of the focused pane**, not as a new tab. Direction is chosen automatically so the grid stays balanced: split horizontally when the focused pane is wider than tall, vertically otherwise (measure the pane element; fall back to alternating by depth when unmeasurable).
- A secondary "New tab" action (menu item or ⌘T-style command) still exists for people who want a separate tab; `new-tab` shortcut keeps working.
- Dragging a pane header onto another pane's edge moves it there (already present) and additionally onto the **empty tab-panel background** appends it as a new split of the last leaf. Drag affordance is discoverable (grip icon + cursor).
- Header dedupe: the workspace title no longer repeats the branch that is already shown on the right. Tab labels use the agent display name ("Claude Code", "Codex") instead of the raw id.

## Unit Tests
- `src/components/MissionControl.test.tsx`: active filter with 501 idle chats; no Show-all controls; a tile renders the conversation and composer; submit and approval resolve target the tile's session; new session auto-inserts; completed workers drop out despite stale runtimes; layout persists and restores; focus and maximize; minimum tile dimensions; sidebar drops into the empty canvas and all four tile edges; pin persistence, removal, and deduplication; invalid drops; rearranging tiles preserves drafts.
- `src/components/BridgeSidebar.interaction.test.tsx`: an idle chat exports the shared sidebar drag payload without navigating away.
- `src/terminal/layout.test.ts`: `autoSplitDirection` helper (or equivalent) and "append to last leaf" insertion.
- `src/components/TerminalWorkspace.test.tsx`: New Agent splits into the current tab (tab count unchanged, leaf count +1); New tab still creates a tab; drop on empty panel background appends; tab label shows agent display name.
- Existing `AgentFleet.test.tsx`, `BridgeSidebar*.test.tsx`, `navigationHistory.test.ts`, `workWiring.test.ts`, `App.test.tsx` stay green.

## Integration / Functional Tests
- `bunx tsc -b` clean. `bunx vitest run` clean for the files above plus `src/App.test.tsx`.

## Smoke Tests
- Preview mode (`vite --port 1468`, accept model setup, dark class): sidebar shows Agent Fleet then Mission Control; Mission Control shows the mock working orchestrator as a live tile with a composer; Agent Fleet "New Agent → Claude Code" splits the current tab.

## E2E Tests
- N/A — native desktop automation is not configured; preview-mode smoke plus component tests cover the boundaries.
