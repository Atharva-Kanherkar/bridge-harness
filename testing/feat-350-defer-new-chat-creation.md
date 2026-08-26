# feat/350 Defer new-chat creation until first submit — Test Contract

Locked before implementation. Source of truth for issue #350: "New chat creates and
persists an empty session before you type anything." New chat must open an *unstarted
draft*; the session row (and its scratch dir / adapter process, already lazy) must not
exist until the first message is submitted.

Scope note from research: **frontend-only.** The backend already lazily creates the
scratch dir and adapter process inside `start_chat`/`start_session`; only the SQLite
`sessions` row is eager, and it is created in exactly two frontend call sites —
`openNewChat` (`bridgeApi.createChat`) and `newWorkspaceSession`
(`bridgeApi.createWorkspaceSession`). No Rust change is required. `start_chat` still
requires the row to pre-exist, so creation is *moved* to first submit, not removed.

## Functional Behavior

- **Deferred entry points** — all resolve a draft, create nothing:
  - Rail "New chat" button (`onOpenNewChat` → `startChatInCurrentRepo`).
  - Per-project / current-repo new chat.
  - Welcome composer (already deferred for the *composer submit*; its **worktree
    toggle** must stop creating eagerly too).
  - `$harness …` shortcut (`openHarnessShortcut`): carries text + handoff brief into a
    fresh chat and **creates on send** (text present ⇒ immediate submit).
- **The draft** holds the choices made before sending: `harness`, `model`,
  `workspaceId` (null ⇒ non-workspace direct chat), `createWorktree`, and an optional
  `carryFromSessionId` for the handoff brief. Harness/model are captured at draft-open
  time so the carry-over from the current direct chat survives deselection.
- **On submit** the session is created with exactly the draft's choices, the first
  message lands in it, and nothing is dropped or double-sent:
  - `workspaceId` set ⇒ `createWorkspaceSession(workspaceId, createWorktree)`.
  - `workspaceId` null ⇒ `createChat(harness, model, null)`.
  - handoff carry (if `carryFromSessionId`) runs before the created id is selected.
- **Navigating away** from an unstarted draft (selecting another session, changing
  view) discards it silently — no row, no scratch dir, no orphan to clean up.
- **Eager paths unchanged**: rewind, fork, retarget (`retargetWorkspace` →
  `newWorkspaceSession`), the orchestrator "+" dialog, and worker creation still create
  eagerly. `retargetWorkspace`'s stop-the-empty-previous cleanup keeps working.

## State transitions

- New chat click → `newChatDraft` set, `selectedSessionId` cleared, `view =
  "workspace"` → Welcome (draft surface) renders. No `createChat`/`createWorkspaceSession`.
- Draft + submit(text) → exactly one create call → select created id → first message
  sent via the existing pending-message path → `newChatDraft` cleared.
- Draft + select another session / change view → `newChatDraft` cleared, no create.

## Unit / Component Tests (Vitest, `src/App.test.tsx` or colocated)

- `NewChat_DoesNotCreateSessionUntilSubmit` — clicking New chat issues **no**
  `createChat`/`createWorkspaceSession` call; state.sessions length unchanged.
- `NewChatDraft_DiscardedOnNavigateAway` — open draft, select an existing session:
  still no create call; rail/session count unchanged.
- `Draft_FirstSubmitCreatesExactlyOneSession` — open draft, submit "hello": exactly one
  create call with the draft's harness/model/workspace, message delivered to it.
- `Draft_RapidDoubleSubmitCreatesOne` — two synchronous submits ⇒ one create call
  (guarded by `newChatPendingRef`).
- `NewChat_RapidDoubleClickOneDraft` — two synchronous New-chat clicks ⇒ one draft.
- `HarnessShortcut_CreatesOnSend` — `$codex ping` from the composer creates a chat
  pinned to codex, carries handoff, sends "ping"; no draft left dangling.
- `WorktreeToggle_DefersCreation` — toggling worktree in the draft sets
  `createWorktree` and creates nothing until submit; the created session then uses it.

Existing regression guards that must stay green:
- Retarget still stops the previous empty session (`retargetWorkspace`).
- `$harness` handoff brief still projected before first cold start.

## Integration / Functional Tests

- N/A for new cross-service wiring — no protocol/Rust change. Covered by the component
  tests above driving `bridgeApi` (mock) call counts.

## Smoke Tests

- `bun run check` (tsc -b + cargo check) green.
- `bunx vitest run src/App.test.tsx` (new cases) green.
- `bun run test` green.

## E2E / Manual Tests

- Launch app → click New chat → observe composer + greeting, **no new row** in the
  rail; click Projects, come back → still no row.
- New chat → type → Enter → exactly one chat appears, message in it, correct harness.
- New chat → Enter twice fast → one chat, one message.
- `$codex explain this` from an open chat → new codex chat with the question answered
  in context.
- Retarget an unstarted chat to another workspace → previous empty chat is gone, no
  duplicate.
