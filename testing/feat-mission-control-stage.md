# feat/mission-control-stage: test contract

Issue #977. Mission Control's split tree (fixed 420x360 minimums summed over the
tree, an assumed 1.6 aspect, drags that re-split) is replaced by a measured
**stage** of equal tiles plus a **queue** of compact cards, ordered by a flat list.

## Functional Behavior

### Persisted state (`src/components/missionControl/layout.ts`)
- Key stays `bridge.mission-control.layout`. Shape
  `{ version: 2, order, pinnedSessionIds, dismissedSessionIds, expandedId, stageLimit }`.
- A v1 value migrates: `order = leafIds(root)` (left to right, top to bottom),
  `expandedId = expandedLeafId` when it is in `order`, pins kept when in `order`,
  dismissals kept.
- Untrusted input: non-string ids dropped, duplicates removed, `stageLimit` is
  one of `1, 2, 4, 6` or `null`, `order` capped at 64 entries, unknown version → empty.

### Fit (`fitGrid(width, height, n, limit?)`, pure)
- Minimum tile 360x300 CSS px, gap 8px.
- `maxCols = max(1, floor((w + 8) / 368))`, `maxRows = max(1, floor((h + 8) / 308))`.
- `shown = min(n, maxCols * maxRows, limit ?? Infinity)`.
- Picks `cols x rows` with `cols <= maxCols`, `rows <= maxRows`, `cols * rows >= shown`,
  minimising `|ln(tileAspect / 1.15)| + 0.35 * emptyCells`.
- At least one tile always shows when `n > 0`, sized to the container (never a
  minimum larger than the container).
- Unmeasured (width or height 0): everything (up to `limit`) shows, `ceil(sqrt)` columns.

### Board plan (`planBoard(width, height, n, limit?)`, pure)
- If not everything fits, reserve a 256px right rail when `width >= 900`, else a
  36px chip strip on top, then re-fit against what is left.
- Returns `{ cols, rows, shown, queue: "rail" | "strip" | null }`.

### Order
- `displayOrder(order, needsYou)`: needs-you chats first, stable among
  themselves, then the rest in `order`.
- `moveInOrder(order, id, target, "before" | "after")`: only `id`'s position changes;
  an `id` not in `order` is inserted. No-op for `id === target`.
- `reconcileOrder(order, { keep, incoming, replaceable })`: drops ids `keep` rejects,
  then each new incoming id takes the slot of the first `replaceable` id
  (finished and unpinned) or is appended. Existing ids never move.

### Component (`src/components/MissionControl.tsx`)
- Stage is a CSS grid `repeat(cols, minmax(0, 1fr))` x `repeat(rows, minmax(0, 1fr))`,
  `overflow-hidden`, no `minWidth`/`minHeight`. Size from a `ResizeObserver`.
- Resize or zoom re-fits without touching `order`.
- Queue cards (`data-queue-id`): project chip, title, harness mark, status
  (inverted pill when it needs you), one line of current activity (the latest ask).
  Click promotes the chat to the front of `order`. Cards are draggable.
- Header drag reorders: drop on a tile's left half inserts before it, right
  half after it; an insertion line shows where. Tile sizes never change.
- A queue card or sidebar chat dropped on a tile takes that slot; the last stage
  tile spills to the queue. A sidebar drop also pins (as before).
- Dropping on empty board space (or the queue) moves the chat to the end.
- Tile header gets "Move earlier" / "Move later" buttons.
- A chat that finishes its turn stays in place with its done status until closed
  or replaced by a newly active chat. Nothing else moves or resizes.
- Dismissals are pruned to sessions that still exist and are active, so the list
  is bounded.
- Toolbar: needs-you button, working count, pinned count, workflow chips,
  project legend, New chat, tiles control (Auto, 1, 2, 4, 6) setting `stageLimit`.
  Arrange is removed. The separate workflow card above the toolbar is gone.
- Maximize shows one tile on the stage; the rest are in the queue.
- Workers stay hidden by default unless pinned; the visibility setting still live-syncs.

## Unit Tests (`src/components/missionControl/layout.test.ts`)
- `fitGrid` table: n in 1..12 over boards for 720x800, 1024x700, 1380x860,
  2560x1440 windows at zoom 0.67, 1, 1.5: `cols * rows >= shown`,
  `cols <= maxCols`, `rows <= maxRows`, tile width/height `> 0` and
  `cols * tileW + (cols-1)*gap <= width` (fits both ways).
- `fitGrid` respects `limit`; unmeasured fallback.
- `planBoard` reserves rail at >= 900 wide, strip below.
- v1 → v2 migration keeps reading order; garbage input → empty v2.
- `moveInOrder` before/after, unknown id insert, self no-op.
- `displayOrder` puts needs-you first, stable.
- `reconcileOrder`: keeps finished in place, new active replaces first finished
  unpinned in place, appends otherwise, drops rejected ids.

## Integration / Functional Tests (`src/components/MissionControl.test.tsx`, jsdom)
- Mocked `ResizeObserver`: stage count follows reported size (e.g. 4 of 7 at a
  small board), rest are queue cards; resizing re-fits and keeps `order`.
- Queue card click promotes to the stage.
- Header drag onto a tile reorders `order`; no separators; drafts preserved.
- Sidebar drop on a tile inserts at that slot and pins.
- Finished chat stays on the board with a done status; a new active chat
  replaces it in place.
- Needs-you chat is on the stage even when it is last in `order`.
- Tiles control sets `stageLimit` and persists it.
- v1 stored layout loads in the same order.
- Move earlier / later buttons reorder.
- Workflow chips render inside the board toolbar.
- Existing behaviours kept: submit/approve per tile, pin/unpin/close, worker
  visibility, legend highlight, jump to needs-you, new chat menu.

## Smoke Tests
- `bun run build` and `bun run test` green.

## E2E Tests
- N/A as automated tests. Manual: Vite mock mode with 7 seeded active chats at
  1380x860, 1024x700, 2560x1440 and zoom 1.5: stage
  `scrollWidth === clientWidth` and `scrollHeight === clientHeight`, equal tiles.

## Manual / cURL Tests
- No API surface. Reachable from a fresh install: sidebar → Mission Control.
- Screenshots: dark, light, drag state (insertion line).
