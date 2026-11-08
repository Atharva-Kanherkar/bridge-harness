# Contract: desktop notifications read the attention ledger

Branch `feat/attention-desktop`, stacked on `feat/attention-ledger`.
Implements #906, slice 2 of #904. Locked before implementation.

## What this changes

The desktop decides what to notify about by diffing two session-list
snapshots (`src/attentionEvents.ts`). This replaces that with the core
ledger from the previous slice: the app follows `attention/changed`, reads
new rows from a cursor, and turns each into the same toast and OS banner it
shows today. Opening a chat marks it seen.

What people notice:

- A worker finishing no longer pings them; its orchestrator finishing does.
- A worker asking for approval now does ping them (today nothing does,
  because a worker's status never becomes `waiting`).
- A restore, warm-up or resume never reads as "Turn completed".
- Restarting Bridge replays nothing that happened before launch.
- An auto-approved permission (Full access) never flashes a toast.

## Functional behavior

`useAttentionFeed(onArrive)` in `src/useAttentionFeed.ts`:

1. On mount, reads `listAttention({ limit: 1 })` and starts its cursor at
   `latestSequence`. Nothing older than launch is shown.
2. On each `attention/changed`, reads `listAttention({ afterSequence: cursor })`,
   paging while a page is full, then moves the cursor to `latestSequence`.
   Reads are serialized: a hint that lands mid-read triggers one more read
   after it, never two concurrent ones.
3. For each new row: skip it if `seenAt` or `resolvedAt` is set or it has no
   `sessionId`. `finished` and `failed` arrive at once. `needsYou` is held
   1 s, then re-read (`afterSequence: sequence - 1, limit: 1`); it arrives
   only if the same row comes back still unresolved and unseen.
4. Unmount cancels pending holds and the subscription.

`App.tsx`, for each arriving row:

- Builds copy with `attentionCopy(item, session)`. When the session is in
  `state.sessions` the copy uses its live name and harness (a title job may
  have renamed it since the row was written); otherwise the row's own
  `title` and `body`.
- Adds a toast keyed `attention:<sequence>`, replacing any toast for the
  same session (newest per chat wins, like the ledger), keeping the last 3.
- Calls `notifyAttention(headline, detail, { sessionId, approvalEventId })`
  with the row's `approvalEventId`. `notifyAttention` keeps its own rules: no
  banner while Bridge is focused, Linux action buttons when an approval id is
  present.
- Selecting a chat calls `markAttentionSeen(sessionId)`; failures are ignored.

Removed: `src/attentionEvents.ts` and its test, the `replaySessionEvents`
lookup in `notifyWithActions`, and `pendingApprovalEventId` in
`src/nativeNotify.ts` (its only caller was that lookup).

`src/api.ts` gains `listAttention`, `markAttentionSeen` and
`onAttentionChanged`. Outside a backend they use `src/attentionMock.ts`, an
in-memory ledger with the core's supersede and seen rules; a mock turn adds a
`finished` row for a top-level chat so `bun run dev` can show the toast.

## Unit tests

- `attentionCopy.test.ts`: live copy from the session for each kind; row
  copy when the session is unknown; tone per kind.
- `useAttentionFeed.test.tsx` (jsdom):
  - boot rows are not delivered; the cursor starts at `latestSequence`;
  - a `finished` row after a hint is delivered once, and a second hint with
    no new rows delivers nothing;
  - a `needsYou` row resolved within the hold is dropped; one still open
    after the hold is delivered;
  - seen, resolved and session-less rows are skipped;
  - a full page triggers a second read;
  - unmount cancels a pending hold.
- `attentionMock.test.ts`: supersede, `unseenOnly`, `markAttentionSeen`
  counts, listener fires.
- `nativeNotify.test.ts` keeps its `parseNotificationAction` cases.

## Integration / functional tests

- `App.attention.test.tsx` (jsdom), rewritten over a mocked ledger:
  - a `needsYou` row for a worker produces one "Bridge needs you" toast and
    one `notifyAttention` call carrying its `approvalEventId`;
  - clicking the toast opens that chat, dismisses the card and calls
    `markAttentionSeen` for it;
  - a newer row for the same chat replaces its toast;
  - rows present at launch produce nothing.

## Smoke tests

- `bunx vitest run src/useAttentionFeed.test.tsx src/App.attention.test.tsx src/attentionCopy.test.ts src/attentionMock.test.ts src/nativeNotify.test.ts`
- `bun run check`, `bun run build`, `bun run test`

## E2E tests

Mock-mode browser check: open the app at the dev server, send a message in a
chat, and see one "Turn completed" toast that opens the chat when clicked.

## Manual tests

Against a desktop build from this branch (release bundle, launched from
`~/Applications`):

1. A chat finishes while Bridge is in the background: one macOS banner and
   one toast.
2. An orchestrator delegates; a worker finishes: no toast. The orchestrator
   finishes: one toast.
3. Quit and relaunch: no toasts for earlier events.
