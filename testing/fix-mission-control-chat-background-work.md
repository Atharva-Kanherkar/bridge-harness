# fix/mission-control-chat-background-work — Test Contract

## Functional Behavior
- Each Mission Control tile shows the same work shelf the open chat shows above its composer: that chat's pull requests and the background work its agent left running.
- A tile reads only its own chat's work. Two tiles with background tasks never show each other's tasks, and a tile never reads the main chat's global store.
- A tile's PRs come from `githubSessionPrs(tileSessionId)` and stay live on the same poller events as the main chat.
- The shelf is absent when the tile's chat has no PRs and no background work.
- Clicking the shelf opens that chat with the Work pane in the dock (on the PR when it is the only one).
- "Fix it" on a failing PR puts the fix prompt into that tile's own composer draft; it does not send.

## Unit Tests
- `createBackgroundWorkStore` is unchanged; `ChatWorkShelf` accepts an optional `store` and reads only it.
- MissionControl.test.tsx:
  - "shows each tile's own pull requests on its shelf"
  - "shows each tile's own background work, not another tile's"
  - "draws no shelf when a tile has no PRs and no background work"
  - "opens the chat's Work pane from a tile's shelf"
  - "drafts a PR fix into the tile's composer"

## Integration / Functional Tests
- Existing MissionControl, WorkShelf, ChatPullRequests and useBackgroundTasks suites stay green.

## Smoke Tests
- `bun run build` green; `bunx vitest run` green.

## E2E Tests
N/A: no E2E harness for Mission Control; covered by component tests and a mock-mode screenshot.

## Manual / cURL Tests
- `bun run dev`, open Mission Control with a chat that has a PR and a backgrounded command: the tile shows the shelf; clicking it opens the chat on the Work pane.
