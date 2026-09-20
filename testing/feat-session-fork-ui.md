# feat-session-fork-ui — Test Contract (frontend PR, step 2 of 2 per issue)

Closes #280 (frontend slice). Builds on stack PR 1 (`feat/session-fork`,
PR #660), which shipped `sessions/fork_session`, migration 58 (fork origin
columns), and the `Session.parentSessionId/depth/restorationMode` surface.

## Functional Behavior

1. **Fork affordance on messages.** Every rendered user and assistant message
   with a forest `entryId` gets a hover action "Fork from here" (next to the
   existing "Remember this", same visual register). Clicking it opens the fork
   dialog pre-seeded with the session id and that entry id. No entry id → no
   affordance. Hidden when `readOnly`.
2. **Fork dialog** (`src/components/ForkDialog.tsx`):
   - Title input, defaulting to nothing (backend derives "Fork of …").
   - Worktree choice as a segmented control: **Share the workspace** (default)
     vs **New worktree and branch**, with the consequence stated in one line
     ("two sessions writing the same files" / "isolated worktree + branch").
   - Submit calls `bridgeApi.forkSession(sessionId, entryId, title, null,
     null, policy)`; on success closes and offers the fork immediately via
     `onForked(forkId)` — the caller (App) switches to it. Errors surface
     inside the dialog, not as a toast goodbye.
   - Harness/model inheritance is silent (no controls) in this slice — the
     backend inherits; overrides land with PR 3+.
3. **Rewind wiring.** The hover actions gain "Rewind to here" (same
   affordance, same entry id) calling `bridgeApi.activateSessionEntry`.
   Because rewind discards the branch above the head, it requires a
   confirmation naming the entry, phrased concretely ("This makes everything
   after *<snippet>* inactive — files are not changed."). Rewind is only
   offered on leaf entries (an entry that is already inactive is not
   rewindable).
4. **Sidebar breadcrumbs.** A session row whose `parentSessionId` is set
   shows a hairline "forked from *<parent name>*" line under the title with a
   small jump-to-parent action that calls the existing `onOpenSession`
   handler. The parent name comes from the sibling session row in the same
   list; unknown parents render as "session".
5. No backend changes in this PR. Everything here drives the PR-1 seam.

## Unit Tests (Vitest, colocated)

- `ForkDialog.test.tsx`
  - renders the pre-seeded session/entry and default "share" policy
  - submit calls `forkSession` with `{ sessionId, entryId, title, null, null,
    "shared" }` and reports the fork via `onForked`
  - selecting "new worktree" submits `"new"` and shows the consequence line
  - a rejected fork keeps the dialog open and shows the error
- `AgentConversation.test.tsx` (additions)
  - a message with an entry id reveals "Fork from here" and "Rewind to here"
    on hover; clicking each fires the matching callback with `(session,
    entryId)`
  - streaming messages, messages without an entry id, and `readOnly`
    transcripts show neither affordance
  - rewind callback is not invoked until the confirmation is accepted
- `BridgeSidebar.test.tsx` (additions)
  - a chat with `parentSessionId` renders the "forked from" line and a
    jump-to-parent button that calls `onOpenSession(parentId)`
  - chats without a parent render as today (no regression)
- `App.test.tsx` (addition)
  - forking from a message opens the dialog; accepting it switches to the
    fork's session id

## Integration / Functional Tests

- Fork → forest: App-level test drives the mock `bridgeApi.forkSession`,
  asserts `state.sessions` contains the fork with `parentSessionId` and that
  the sidebar next render shows the breadcrumb.
- Rewind → forest: accepted rewind calls `activateSessionEntry(sessionId,
  entryId)` and the transcript re-renders from the returned snapshot.

## Smoke Tests

- `bun run test` (full Vitest + cargo) green; `bun run check` green;
  `bun run build` green.

## E2E Tests

N/A — desktop app; covered by the component tests above and the manual pass.

## Manual / cURL Tests

Manual pass (documented for the reviewer): `bun run tauri dev`, open a chat,
hover a message → Fork from here → dialog shows both policies → share → the
fork opens; sidebar shows "forked from"; rewind on a leaf asks and rewinds.