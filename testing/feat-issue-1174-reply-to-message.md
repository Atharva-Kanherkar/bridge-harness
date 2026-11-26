# Reply to an earlier agent message: test contract

Reply on a settled agent message makes the next prompt answer that message, as
T3 Code's reply does. The chat's head, branches and worktree do not change.

## Functional behavior

- Reply shows on settled assistant messages only: not on user messages,
  streaming messages, read-only transcripts or previews. It is in the action bar,
  visible on the latest reply and shown on hover for older ones.
- Clicking Reply shows a "Replying to" banner above the composer with the
  message's opening, focuses the composer, and sends nothing.
- A second Reply replaces the target. ✕ on the banner clears it, and so does Esc
  in an empty composer.
- Sending puts a one-line marker comment, then the excerpt as `> ` lines (cut at
  1,500 characters with an ellipsis line), a blank line, and the user's words
  before the prompt. Slash commands and harness or agent shortcuts do not carry it.
- A successful send clears the target. A failed send returns the user's words and
  the target to the chat they came from, never the raw quote.
- The sent bubble shows a "Replying to" card instead of the raw quote. Clicking
  the card scrolls to the original message and highlights it. The marker gives the
  entry id; a target taken before the entry landed resolves its id at send time.
- The target is part of the chat's draft and survives a switch to another chat.
- No wire change: the marker lives in the stored prompt, so the card still shows
  after a reload. Messages without the marker render as their text.

## Unit tests

- `src/replyTo.test.ts`: excerpt cut, surrogate safety, blank lines, prompt
  format, round-trip parse, unsafe ids, copy text, preview, entry resolution.
- `src/composerDrafts.test.ts`: reply kept, consumed, restored per draft.

## Component and app tests

- `src/components/AgentConversation.reply.test.tsx`: where Reply shows, the
  bubble card, the jump back, plain quotes left alone, copy.
- `src/components/AgentConversation.memo.test.tsx`: a new `onReply` reaches the row.
- `src/App.reply.test.tsx`: banner, replace, ✕, send format, failure restore,
  Esc, slash commands, per-chat target.

## E2E

- A browser tab on a scratch `bridged` (web client): a real Claude Code chat,
  Reply on an older answer, send, and the agent answers about that answer.
  Screenshots in dark and light, and a recording of the flow.
