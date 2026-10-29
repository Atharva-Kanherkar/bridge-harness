# perf/instant-transcript-load — Test Contract

Opening a long chat must show its newest messages fast and keep rendering correct
while the rest of the history arrives. This contract was written before any
implementation and is the definition of done for the branch.

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
- The cache cannot retain every forest forever: it holds at most 8 entries
  arrays (the most recently used), and leaf-id variants per array are bounded
  too. Evicted entries must not break later calls.

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
- Hydration never unmounts a row that is already on screen: the boundary is a
  count of hydrated *leading* rows, so new tail rows (streaming, optimistic
  bubbles) do not shift it.
- Hydration is skipped entirely when there are at most `INITIAL_TRANSCRIPT_ROWS`
  rows.
- Full mount happens immediately when `highlightEntryId` is set, because a
  search/recall jump must land on a row that exists.
- Full mount happens immediately when the chat has a remembered scroll position
  that is not pinned to the bottom, because a partial transcript cannot restore
  that offset.
- Hydration uses `requestIdleCallback` when available and a `setTimeout`
  fallback otherwise (Safari/WKWebView and jsdom have no `requestIdleCallback`).
- Reset is keyed by session, so switching chats starts from the newest rows
  again and pending callbacks for the previous session cannot add rows to the
  new one.
- Hydrated (older) rows mount without entrance animation. Rows inside
  `AnimatePresence` keep their existing entrance/exit behavior.
- `ScrollFollow`'s signature includes the revealed row count, so a reader at the
  bottom stays at the bottom while older rows are added above, and a reader who
  scrolled up is not moved.
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

## Unit Tests

`src/transcript/projectionCache.test.ts`

- `caches by entries identity and leaf id` — second call returns the same
  reference; `projectSessionConversation` is called once.
- `re-projects when the leaf changes` — different leaf, different result,
  projected twice.
- `re-projects when the entries array identity changes` — equal contents, new
  array, projected twice.
- `matches the uncached projection` — for a 200-entry branch, cached and
  uncached results deep-equal.
- `evicts the oldest entries arrays` — 10 distinct arrays; the first is
  re-projected after eviction, the ninth is still cached.
- `bounds leaf variants per array` — more than the per-array leaf cap evicts in
  LRU order without changing results.

`src/transcript/reducer.test.ts`

- `settles thinking without walking the whole fold` — replay with 20,000
  non-thinking items and 3 streaming thoughts; count `Map.prototype.values`
  traversals across the settling frames and assert the fold's full size is never
  traversed. Correctness: all 3 thoughts completed, candidates empty.
- `leaves settled reasoning and non-reasoning candidates alone`.

`src/components/transcriptWindow.test.ts`

- `initial window keeps the newest rows` — `initialHydratedCount(1500) = 1468`,
  `initialHydratedCount(10) = 0`.
- `forced window reveals everything` — `initialHydratedCount(1500, true) = 1500`.
- `hydration grows by one chunk and clamps` — `nextHydratedCount(1468, 1500) =
  1500`, `nextHydratedCount(0, 10) = 10`.
- `the hydrated prefix is stable when tail rows arrive` — with 68 of 100 rows
  hydrated, adding two tail rows keeps the first 68 rows hydrated, so no
  already-mounted row is unmounted.

`src/components/useInViewport.test.tsx`

- `returns visible when IntersectionObserver is unavailable`.
- `starts hidden, then visible after an intersecting entry`.
- `disconnects on unmount` and ignores a late entry after unmount.
- `stays visible after the element leaves` (no flip back to hidden).

`src/components/Markdown.test.tsx` (extended)

- `does not colorize a code block that has never been visible` — with a mocked
  `IntersectionObserver`, no `colorizeCode` call after the debounce.
- `colorizes once the block intersects` — trigger the callback, advance the
  debounce, one call, coloured HTML swapped in.
- `keeps its colour after leaving the viewport` — no second call, no reset.

## Integration / Functional Tests

`src/components/AgentConversation.history.test.tsx` (extended)

- `mounts only the newest rows first` — a 300-row history renders
  `INITIAL_TRANSCRIPT_ROWS` rows on the first commit.
- `hydrates older rows in idle slices` — advancing the idle queue mounts the
  rest in `TRANSCRIPT_HYDRATE_CHUNK` steps, never exceeding the chunk per step.
- `mounts everything at once for a highlighted entry` — with
  `highlightEntryId` pointing at the oldest row, that row exists on the first
  commit.
- `mounts everything at once for a remembered position`.
- `keeps a streaming tail when older rows hydrate` — adding a tail row between
  slices does not unmount an already-hydrated row.

`src/components/AgentConversation.scroll.test.tsx` (extended)

- `stays pinned to the bottom across hydration` — with the viewport at the
  bottom, each hydration slice leaves `scrollTop` at the new `scrollHeight`.

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
