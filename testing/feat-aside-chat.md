# feat/aside-chat — Test Contract

Field request: `$harness <message>` typed inside an open chat should behave
like a delegation the *user* makes — a standalone temp agent consulted from
the conversation you are in — not a navigation. Today it creates the sibling
chat correctly (with the projected handoff brief) but then yanks the view to
it; the person loses the chat they were reasoning in.

The product name for it: an **aside**. Same structured context injection as
today (`carry_session_handoff` is untouched), but the conversation stays put
and the aside opens as a floating panel over it.

Locked before implementation. Frontend-only: the backend already has
everything this needs.

## Functional Behavior

- `$codex is this right?` typed in the composer of an **open session**:
  1. creates a direct chat pinned to that harness (title derived from the
     question, truncated), exactly as today;
  2. carries the handoff brief from the current session, exactly as today;
  3. sends the question as its first message;
  4. opens the new chat in an **aside panel** floating over the current
     conversation — the selected session does not change, the sidebar
     selection does not move, the underlying transcript stays visible.
- The Welcome screen keeps today's behavior: with no session open there is
  nothing to stay in, so the shortcut still becomes the new chat.
- The aside session is a real, durable, standalone chat. It appears in the
  sidebar like any other (no hidden kind: a conversation a person is having
  must stay reachable after the panel closes).
- The panel (`AsideChat.tsx`):
  - a scrim over the session pane; a floating glass panel (`u-glass-popover`
    ladder, rounded, bounded width/height) with the app's motion vocabulary;
  - header: the harness's `HarnessMark` in its tint, the harness · model
    label, the aside's title, a "Open as chat" promote button, a close ✕;
  - body: the real `AgentConversation` (live events filtered to the aside's
    id, optimistic pending rows, approval resolution wired to the aside's id
    — not to the selected session's);
  - footer: a compact composer; ⏎ sends follow-ups into the aside through the
    same prepare/start/submit path the main composer uses;
  - Escape and the scrim close it; closing never ends or archives the session;
    "Open as chat" selects the aside as the active session and closes the
    panel.
- Reopening later happens through the sidebar as a normal chat; the panel is
  the delegation surface, not a second home for the session.
- No em dashes in any user-visible string. No hedging copy.

## Unit Tests

- `harnessShortcut.test.ts` unchanged (parsing untouched).
- `AsideChat.test.tsx` (new, jsdom):
  - renders the harness mark with its tint, the label, and the title
  - Escape and scrim call `onClose`; the panel body does not
  - the promote button calls `onPromote`
  - typing and ⏎ calls `onSend` with the text and clears the input
  - approval cards inside resolve through the aside session id
- `App.test.tsx`:
  - `$<mock harness> question` sent from an open session leaves the selected
    session unchanged and renders the aside panel with the question pending
  - the aside session exists in state afterwards (sidebar-reachable)
  - promote selects the aside session and unmounts the panel

## Integration / Smoke

- `bun run check`, `bun run test`, `bun run build` all green.
- Live in mock mode: shortcut from an open chat opens the panel over the
  conversation; the underlying chat stays; promote navigates.

## E2E

N/A — desktop app.

## Manual Tests (reviewer)

1. In a real chat with history, `$claude summarize where we are` → panel opens
   over the chat, Claude answers in it, the original chat never moves; the
   handoff brief row is visible in the aside's transcript.
2. Follow-up typed in the panel reaches the aside agent.
3. Escape closes; the aside remains in the sidebar; reopening it is a normal
   chat.
4. "Open as chat" promotes it to the active session.
5. `$mistyped-harness msg` still reports the unavailable-harness error in the
   main view.
