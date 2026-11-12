# perf/instant-transcript-load — Test Contract

Opening a long chat must show its newest messages fast and keep rendering correct
while the rest of the history arrives. This contract was written before any
implementation and is the definition of done for the branch.

> Review amendment: the review of the first implementation found three
> correctness bugs and two performance gaps. Those corrections are folded into
> the behavior and test lists below; they do not widen the product scope.

## Functional Behavior

### 1. One projection per unchanged forest

`projectSessionConversation(entries, activeLeafId)` stays pure and unchanged.
A new wrapper, `projectSessionConversationCached`, is used by `App.tsx` where the
same forest is projected repeatedly (pending reconciliation on every streaming
flush, send watermark, retry payload, conversation-started check).

- Same `entries` array identity and same `activeLeafId` returns the **same array
  reference** and does not re-project.
- A different `activeLeafId` on the same entries re-projects.
- A different `entries` identity re-projects, even when the contents are equal.
- The result is always deep-equal to `projectSessionConversation` for the same
  inputs.
- The cache holds the projection through a `WeakMap` keyed by the entries array,
  so a forest the app drops can be collected; there is no outer LRU pinning old
  forests. Leaf-id variants per array are bounded at 4.
- Cached arrays are frozen in development so a caller cannot mutate a projection
  another reader shares.

### 2. Settling thoughts is proportional to unsettled thoughts

`settleThinking` currently walks every item in the fold on each
`turn.completed` / settling `session.lifecycle`, which is quadratic over a long
turn-heavy replay.

- A settle must visit only the fold's thinking candidates, never the whole item
  map.
- Every streaming / inProgress reasoning item still settles to `completed`, and
  is removed from `thinkingCandidates`.
- Reasoning items that were already settled are untouched.
- Non-reasoning candidates are left alone.

### 3. Progressive transcript mount

- First commit of a chat mounts only the newest `INITIAL_TRANSCRIPT_ROWS` (32)
  rows. Older rows hydrate in idle slices of `TRANSCRIPT_HYDRATE_CHUNK` (48)
  rows until all are mounted.
- Hydration never remounts a row that is already on screen: the window is an
  index into the head of the transcript, so new tail rows (streaming,
  optimistic bubbles) do not move it.
- Hydration is skipped entirely when there are at most `INITIAL_TRANSCRIPT_ROWS`
  rows.
- Full mount happens immediately when `highlightEntryId` is set, because a
  search/recall jump must land on a row that exists. Clearing the highlight
  afterwards does not collapse the window it opened.
- Full mount happens immediately when the chat has a remembered scroll position
  that is not pinned to the bottom, because a partial transcript cannot restore
  that offset. That answer is captured once per chat, so scrolling up later in
  the session does not turn into a full mount.
- Hydration uses `requestIdleCallback` when available and a `setTimeout`
  fallback otherwise (Safari/WKWebView and jsdom have no `requestIdleCallback`).
- Reset is keyed by the chat the rows belong to, and seeding waits until the
  forest matches the selected session. Selecting a chat commits the session one
  commit before its forest, and seeding from the previous chat's row count is
  how a ten-row chat used to open a three-hundred-row one fully mounted. A
  remembered-offset full mount waits for the new forest too; applying it on the
  mismatch commit would mount every row of the chat being left.
- A branch that shrinks past the stored start clamps the start into state, so
  the next idle slice advances from the rows actually mounted instead of
  stalling on an index that no longer exists.
- Hydrated (older) rows mount without entrance animation. Rows inside
  `AnimatePresence` keep their existing entrance/exit behavior. The opening
  window's instant boundary is captured once per chat; tail growth does not move
  it and flip an already-animated row to instant.
- `ScrollFollow`'s signature includes the revealed row count, so a reader at the
  bottom stays at the bottom while older rows are added above. A reader who
  scrolled up keeps the same text: the scroller disables native scroll anchoring
  (WebKit does not implement it) and a layout effect adds the height that
  arrived above them to `scrollTop` before paint.
- The `entryWindow` notice for a trimmed snapshot is unchanged.

### 4. Code blocks colorize only when visible

- `CodeBlock` renders escaped plain text immediately, as today.
- It does not call `colorizeCode` until the block has been in the viewport.
- Once visible, the existing `COLORIZE_DEBOUNCE_MS` debounce and stale-result
  guard still apply.
- A block that scrolls out keeps the colour it already has; it is not
  re-tokenized and not reset to plain.
- When `IntersectionObserver` is unavailable, the block counts as visible
  immediately, so behavior is unchanged in environments without it.
- The observer's root is the nearest scrollable ancestor (the transcript
  scroller), not the window. A positive `rootMargin` cannot grow a nested
  scroller's clip, so the default viewport root gave no look-ahead at all.
- Fences under the same root share one observer per root and margin, instead of
  creating one observer per fence.
- `MAX_HIGHLIGHT_CHARS` and the plain fallback are unchanged.

### 5. Behavior that must not change

- The newest row is visible, and the viewport pinned to the bottom, on the first
  painted frame after a chat opens.
- `data-entry-id` / `id="forest-entry-…"` are present on every rendered row, so
  scroll-to-entry still works.
- `AgentConversation`'s projection still goes through the pure
  `projectSessionConversation` (existing tests spy on it and expect one call per
  unchanged entries array).
- No protocol, backend, or snapshot-window changes in this PR.

### 6. Hydration render cost is proportional to rows that arrive

- `TranscriptEntry` and `ItemView` are memoized, so an idle slice re-renders
  only the rows it adds. Rows already mounted bail out of the React render.
- `AgentConversation`'s default `events`, `pendingMessages`,
  `pendingAdoptions`, and `onAnswerQuestion` values are module constants. A
  literal default is a new identity on every internal render and would defeat
  the memo comparison for every row.
- The projection cache is keyed by entries identity through a `WeakMap`, so a
  dropped forest's projection does not keep it alive.
- Thinking candidates retain `inProgress`; assistant candidates do not. Settling
  still finds every unsettled thought, without growing the assistant candidate
  set with settled rows.

## Unit Tests

`src/transcript/projectionCache.test.ts`

- `caches by entries identity and leaf id` — second call returns the same
  reference; `projectSessionConversation` is called once.
- `re-projects when the leaf changes` — different leaf, different result,
  projected twice.
- `re-projects when the entries array identity changes even with equal contents`
  — new array, projected twice.
- `matches the uncached projection` — for a 200-entry branch, cached and
  uncached results deep-equal.
- `bounds leaf variants per array in LRU order` — more than the per-array leaf
  cap evicts in LRU order without changing results.
- `freezes the shared projection in development` — a cached array cannot be
  mutated in place.

`src/transcript/reducer.test.ts`

- `settles turns without walking the whole fold each time` — a 200-turn,
  20,200-row replay; count `Map.prototype.values` traversals across the
  settling frames and assert the fold's full size is never traversed once per
  turn. Correctness: every reasoning row is completed and the fold holds 20,200
  rows.
- `settles streaming thinking when the turn ends` and `settles a thinking row a
  provider left in progress` — `inProgress` stays a thinking candidate until it
  settles.

`src/components/transcriptWindow.test.ts`

- `keeps only the newest rows in the opening window` — `initialWindowStart(1500) = 1468`,
  `initialWindowStart(10) = 0`.
- `mounts everything when the window is forced` — `initialWindowStart(1500, true) = 0`.
- `moves the start one chunk toward the head per slice` —
  `nextWindowStart(1468) = 1420`, `nextWindowStart(20) = 0`.
- `keeps the window start stable when tail rows arrive` — the start is seeded
  at 68 for a 100-row chat and stays 68 as the live tail grows, so no
  already-mounted row is remounted.

`src/components/transcriptWindow.hook.test.tsx`

- `opens on the newest rows` and `mounts everything once a chat is short
  enough`.
- `keeps showing the newest rows when a rewind shrinks the branch`.
- `does not seed from the previous chat's rows while the forest catches up` —
  a ten-row chat followed by a three-hundred-row forest starts at 268, not 0.
- `drains hydration to the first row after a shrink past the stored start` —
  a 600-row window clamped to a 200-row branch reaches row 0.
- `keeps the old window on the mismatch commit when the next chat opens full` —
  a remembered-position mount does not expand the outgoing chat's rows.
- `keeps the opening boundary fixed when the tail grows` — `instantBefore` does
  not move as new tail rows arrive.

`src/components/useInViewport.test.tsx`

- `returns visible when IntersectionObserver is unavailable`.
- `starts hidden, turns visible on an intersecting entry, and stays visible` —
  leaving the viewport does not flip it back to hidden.
- `disconnects on unmount and ignores a late entry`.
- `measures the look-ahead against the transcript scroller and shares one
  observer` — `root` is the scroll parent, `rootMargin` is 200px, and two
  fences under it use one observer.

`src/components/Markdown.test.tsx` (extended)

- `does not colorize a code block that has never been visible` — with a mocked
  `IntersectionObserver`, no `colorizeCode` call after the debounce.
- `colorizes once the block intersects` — trigger the callback, advance the
  debounce, one call, coloured HTML swapped in.
- `keeps its colour after leaving the viewport` — no second call, no reset.

## Integration / Functional Tests

`src/components/AgentConversation.history.test.tsx` (extended)

- `mounts only the newest rows on the first commit` — a 300-row history
  renders `INITIAL_TRANSCRIPT_ROWS` rows on the first commit.
- `hydrates older rows one idle slice at a time` — advancing the idle queue
  mounts the rest in `TRANSCRIPT_HYDRATE_CHUNK` steps, growing the mounted row
  count by at most one chunk per step.
- `mounts every row at once for a highlighted entry` — with
  `highlightEntryId` pointing at the oldest row, that row exists on the first
  commit.
- `keeps the history mounted after a search highlight clears`.
- `keeps an already-hydrated row mounted when a live row arrives` — adding a
  tail row between slices does not unmount hydrated history.
- `opens a long chat on its newest rows when switching from a short one` —
  session A (10 rows) to B (300 rows) mounts 32 B rows on the first commit.
- `finishes hydrating after the branch shrinks past the stored start` — a
  same-chat 600 -> 200 shrink reaches `b0` after the idle queue drains.

`src/components/AgentConversation.scroll.test.tsx` (extended)

- `stays pinned to the bottom across hydration` — with the viewport at the
  bottom, each hydration slice leaves `scrollTop` at the new `scrollHeight`.
- `keeps a reader who scrolled up on the same text while rows hydrate above` —
  the offset grows by exactly the height added above, because native anchoring
  is off, and the remembered offset is updated.
- `mounts the whole transcript when reopening at a remembered position` — the
  remembered-position mount waits for the chat's own forest before placing.
- `waits for the chat's own history before placing` — a session switch does not
  place the new chat using the outgoing chat's rows.

`src/components/AgentConversation.test.tsx` (unchanged, must stay green)

- One `projectSessionConversation` call per unchanged entries array.

## Smoke Tests

- `bun run build` is green (`tsc -b` + `vite build`).
- `bun run test` is green.
- `bun run check` is green (Rust untouched, but the workspace must still check).

## E2E Tests

N/A — this app has no end-to-end harness; the integration tests above drive the
real components in jsdom. Manual verification below covers the desktop path.

## Manual / cURL Tests

Open a chat with at least several hundred events (use a long session from the
sidebar, or seed one) and check:

1. **Cold open** — the newest message is on screen immediately, and the viewport
   is at the bottom, with no flash of the older history.
2. **Long scroll** — scroll to the top; all older rows are present, in order,
   with no duplicates and no missing `data-entry-id`.
3. **Search jump** — use session search / recall to jump to an entry near the
   top of a long chat; the target row is highlighted and scrolled into view on
   the first attempt.
4. **Reopen at position** — scroll up, switch chats, switch back; the previous
   offset is restored.
5. **Streaming** — send a prompt in a long chat; the reply streams without
   stutter, and the pending bubble does not re-projection-stall the UI.
6. **Thinking rows** — a turn with thinking settles its reasoning rows to done;
   nothing stays spinning after the turn completes.
7. **Code blocks** — open a long chat containing fenced code; visible blocks
   colour in, and scrolling does not re-trigger colorization of blocks already
   passed.
