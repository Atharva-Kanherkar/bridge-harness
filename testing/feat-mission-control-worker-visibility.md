# feat/mission-control-worker-visibility — Test Contract

## Problem

Mission Control auto-surfaces every active session, including workers a
delegating orchestrator spawns. When several workers are active at once the
grid fills with worker tiles, and the user cannot tell which chat is the
orchestrator they're actually steering. There is also no way to dismiss a
tile that is still active (the only "X" today only appears on a pinned tile
once it goes idle).

## Functional Behavior

1. **Close button on every tile.** Each Mission Control tile header gets a
   "Close chat" button (an `X`, `aria-label="Close chat"`) that is always
   present, regardless of the session's status (active or idle) or pinned
   state. Clicking it:
   - Removes the tile from the grid immediately.
   - Clears the session's pin, if it was pinned.
   - Does **not** stop the underlying session/worker — that remains the
     separate "Stop worker" action. Closing is a view action only.
   - The dismissal is remembered (persisted in the Mission Control layout)
     so the tile does not reappear on its own while the session stays
     active. It reappears only if the user explicitly brings it back
     (dragging the chat in from the sidebar again, or pinning it again).
   - Pin/unpin (`Pin`/`PinOff`) keeps its existing, separate meaning: it
     toggles whether the tile survives after the session goes idle. It no
     longer also acts as a close action for idle+pinned tiles — the new
     Close button covers that uniformly.

2. **"Show worker chats" setting, off by default.** A new local (frontend
   only, `localStorage`-backed) preference controls whether sessions with a
   `parentSessionId` (workers) are ever auto-surfaced in Mission Control.
   - Default: **off** — Mission Control only auto-surfaces sessions without
     a `parentSessionId` (orchestrators / plain chats). Worker chats spawned
     from delegation stay out of the grid unless the user turns the setting
     on.
   - When a worker chat is explicitly pinned (dragged in from the sidebar,
     or pinned from its own tile before the setting existed), it still
     shows regardless of the setting — pinning is an explicit user choice
     and overrides the default filter.
   - Turning the setting on restores the previous behavior: every active
     session, worker or not, auto-surfaces.
   - The control lives in Settings → Appearance, a `Switch` row labeled
     "Show worker chats in Mission Control", persisted under
     `bridge.missionControl.showWorkerChats`.

## Unit Tests

`src/components/missionControl/layout.test.ts` (extends the existing
coverage in `MissionControl.test.tsx` / any dedicated layout test):
- `parseLayout` accepts and dedupes a `dismissedSessionIds` array; ignores
  non-string entries; defaults to `[]` when absent (old saved layouts).

`src/missionControlSettings.test.ts` (new):
- `readShowWorkerChatsInMissionControl` returns `false` when unset.
- `writeShowWorkerChatsInMissionControl(true)` round-trips through
  `readShowWorkerChatsInMissionControl`.
- Malformed/absent storage values fail closed to `false`.

## Integration / Functional Tests

`src/components/MissionControl.test.tsx`:
- Closing a non-pinned, active tile removes it from `tiles()` immediately
  and it does not reappear on the next render with the same sessions.
- Closing a pinned tile clears the pin (`savedLayout().pinnedSessionIds`)
  and removes the tile.
- Closing a tile does not call `onStopWorker` or `interruptTurn`.
- With the setting off (default), a worker session (`parentSessionId` set)
  that is active does not appear in `tiles()`, while a plain/orchestrator
  session does.
- A worker session that is pinned still appears even with the setting off.
- With the setting on, an active worker session appears in `tiles()`
  alongside the orchestrator.
- Dragging a previously-closed chat back in from the sidebar clears its
  dismissal and shows it again.

`src/components/settings/AppearancePage.test.tsx` (new or extended):
- Renders the "Show worker chats in Mission Control" switch, unchecked by
  default.
- Toggling it writes through to `localStorage` under
  `bridge.missionControl.showWorkerChats` and flips the switch's
  `aria-checked`.

## Smoke Tests

- `bun run check` passes (tsc -b + cargo check).
- `bunx vitest run src/components/MissionControl.test.tsx src/missionControlSettings.test.ts src/components/settings/AppearancePage.test.tsx` passes.

## E2E Tests

N/A — no Tauri/backend round trip is involved; this is a frontend-only
filter and localStorage preference. Manual verification happens via
`bun run dev` against mock data (see Manual section).

## Manual / cURL Tests

1. `bun run dev`, open Mission Control with a mocked orchestrator + worker
   session active. Confirm only the orchestrator tile auto-appears.
2. Open Settings → Appearance, flip "Show worker chats in Mission Control"
   on. Confirm the worker tile now appears without a reload.
3. Click the new `X` ("Close chat") on an active tile. Confirm it
   disappears and does not come back while its status stays active.
4. Drag that same chat back in from the sidebar. Confirm it reappears.
5. Pin a worker tile with the setting off; confirm it stays visible; click
   Close; confirm the pin clears and the tile disappears.
