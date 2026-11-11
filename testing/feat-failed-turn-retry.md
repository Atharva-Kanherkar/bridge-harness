# feat/failed-turn-retry: Test Contract

A chat whose last turn failed (a provider error such as no credits, a rate
limit or an outage) offers two actions on the error card: **Retry** resends the
failed message as it was, and **Edit** puts it back in the composer to add to.

## Functional Behavior

- The actions sit on one card only: the newest error row of the transcript,
  when no assistant reply came after it and a user message came before it.
  That user message is the failed prompt.
- An earlier error that the chat has since moved past (a later assistant reply
  or a later user message) shows no actions.
- No actions while a turn is running, while an optimistic send is still
  pending, in read-only history, in preview, or on a degraded history entry
  ("Unavailable history entry").
- **Retry** sends the failed prompt's text and its image attachments through
  the normal send path (`sendPrompt`), so slash commands, cold start and
  failure restore all behave as for a typed send. Nothing typed in the composer
  is lost.
- **Edit** puts the failed prompt back into the composer ahead of anything
  already typed (blank line between, `mergeFailedSend`), adds its images, and
  focuses the composer. Nothing is sent.
- After Retry the buttons go away (the turn is active / the send is pending);
  if the retried turn fails again, the new error row carries them.
- Usage-limit (warning) cards and error cards both get the actions.

## Unit Tests

`src/failedTurn.test.ts`
- returns the error and the user message it answers when the last turn failed
- skips activity, reasoning and partial assistant text between prompt and error
- returns nothing when an assistant reply follows the error
- returns nothing when a newer user message follows the error
- ignores degraded history entries
- returns nothing when there is no user message before the error

`src/providerLogin.test.ts`
- `userPromptPayload` keeps text and image attachments of a user message
  (existing sign-in retry tests keep passing on the shared helper)

## Integration / Functional Tests

`src/components/AgentConversation.test.tsx` (jsdom)
- the failed last turn's error card shows Retry and Edit; clicking them calls
  `onRetryTurn` / `onEditTurn` with the failed user message
- an earlier, answered error shows no buttons
- no buttons while `working`, while a pending message is in flight, or when
  `readOnly`

## Smoke Tests

- `bun run build` and `bun run test` are green.

## E2E Tests

N/A: the desktop app has no E2E harness for provider failures. Covered by the
manual check below.

## Manual / cURL Tests

- `bun run dev`, open a chat in mock mode, inject a transcript whose last turn
  is an error: the card shows Retry and Edit (screenshot dark and light).
- Edit with a draft already typed: the failed message comes first, the draft
  after a blank line.
