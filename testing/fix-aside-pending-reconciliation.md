# fix/aside-pending-reconciliation — Test Contract

Field report on the merged aside feature: `$claude hi` gets its answer, and the
startup row underneath keeps counting ("85s · Waiting for Claude to answer…")
forever.

Root cause: the optimistic-pending reconciler only looks at the *selected*
session. It filters against `sessionEvents` (the selected session's slice of
the live stream) and the selected session's forest, and it only re-runs when
those change. That was sound while every pending row belonged to the selected
session; the aside broke the assumption. An aside's pending row can never
reconcile, `pendingMessages.length` never reaches zero, `hasPendingWork` stays
true, and the narration row is mounted for the rest of the session's life.

Locked before implementation.

## Functional Behavior

- New pure `undeliveredPending` in `src/conversation.ts`: given the pending
  rows, the **global** live stream, and the selected session's durable user
  texts, it drops every pending row whose text has arrived as a real user
  message **in that row's own session** (live for any session; durable
  additionally for the selected one, whose forest is the only one App holds).
- The App effect uses it and re-runs on the global stream (`agentEvents`),
  not the selected slice, so an aside's arrival is seen at all.
- Behavior for the selected session is unchanged: same sources, same
  trim-and-match rule, same referential no-op when nothing is dropped.

## Unit Tests

- `conversation.test.ts` — `undeliveredPending`:
  - an aside's pending row drops when its user turn arrives in the aside's
    stream, and survives when the same text arrives in a different session
  - the selected session still reconciles through durable texts alone
  - nothing matching returns the same array reference (render stability)

## Integration / Smoke

- `bun run check`, `bun run test`, `bun run build` all green.

## Manual Tests (reviewer)

1. `$claude hi` from a real chat: the reply lands, the narration row leaves
   within its 2s linger, the panel goes quiet. No eternal counter.

## E2E

N/A.
