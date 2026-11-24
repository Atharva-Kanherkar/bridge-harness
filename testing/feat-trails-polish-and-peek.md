# feat/trails-polish-and-peek: Test Contract

Three changes to the Trails UI: the loading state, visual polish, and a trail
peek from the sidebar and the chat header. Frontend only: no Rust, no protocol.

## Functional Behavior

### Loading

- L1. Opening a trail whose summary Bridge already holds (the Trails list, or
  App's trail list passed in) paints its header at once: title, state chip,
  stage, needs-you reason, chat count. The timeline and the rail show
  skeletons until the detail arrives.
- L2. Opening a trail reads the local detail first (`readTrail(id, false)`),
  then re-reads from GitHub (`readTrail(id, true)`) after the local read
  lands. The page never waits on GitHub to paint.
- L3. While any GitHub refresh is in flight for the trail on screen, the
  Refresh button shows a spinner, is disabled and reads "Refreshing from
  GitHub". It stops when that refresh settles, success or failure.
- L4. A reload keeps the last detail on screen: no skeleton after the first
  paint.
- L5. The race rules hold (existing test): a response for a trail the person
  left never lands; an older request for the same trail loses to a newer one,
  errors included.
- L6. A refresh that succeeded but lost to a newer local read triggers one
  local catch-up read, so what GitHub stored still reaches the screen.
- L7. When the first read fails and nothing is on screen, the error shows with
  a Retry button that reads again.
- L8. Every skeleton sits in a labelled `SkeletonRegion` (`role=status`):
  "Loading trail" for the timeline, "Loading the brief" for the rail, and the
  list skeleton is shaped like list rows (unframed).
- L9. With no summary known, the header shows a title skeleton and the back
  link, not a blank page.

### Polish

- P1. Event row: title first; under it one meta line that starts with the
  kind label ("Issue", "Pull request", ...), then the detail (toned), then
  "added automatically". The meta line starts at the title's left edge.
- P2. Times sit in one fixed-width right column on every row, including
  Checks rows that have no remove button.
- P3. Times and token counts use sans `tabular-nums`, not mono. The branch
  chip stays mono (it is code).
- P4. Each brief item in the rail carries its kind's icon (issue, note, pull
  request, chat, message) from its id prefix.
- P5. No new palette classes, hex literals, hand-rolled pulse bars or mono
  times: `src/designSystem.test.ts` stays green.

### Trail peek

- K1. A sidebar chat row whose chat sits on a trail shows a trail button at
  the right of the row while the row is hovered or has focus inside it, and
  while its card is open. A chat on no trail has no trail button.
- K2. Clicking the trail button opens that trail's page.
- K3. Hovering (or focusing) the trail button opens a card after a short delay
  with: the trail's title, state and stage, the needs-you reason when set,
  harness marks and chat count, up to four latest events (icon, title, when),
  up to two open items, and an "Open trail" button.
- K4. The card paints the summary at once; the events show a labelled
  skeleton until `readTrail(id)` (local, no GitHub refresh) answers. A second
  open of the same trail shows the last events at once and re-reads behind.
- K5. A failed read in the card says so in one line; the summary stays.
- K6. "Open trail" in the card opens the trail page and closes the card.
- K7. The chat header's trail chip opens the same card on hover; clicking the
  chip still opens the trail page.

## Unit Tests

- `src/trails.test.ts` (or existing helpers test): `briefKind(id)` maps
  `issue:`, `note:`, `pr:`, `chat:`, `message:` and falls back to `other`;
  `latestEvents(events, n)` returns the newest n, newest first, skipping
  `removed`.
- `src/components/trails/TrailPage.test.tsx`:
  - "renders every event kind with its label" still passes.
  - "puts the kind label and detail on one meta line under the title" (P1).
  - "reserves the remove slot so every time lines up" (P2): a Checks row and
    a Note row both have the slot element.
  - "renders the header from a summary while the detail loads" (L1, L8).
  - "spins Refresh while a GitHub refresh is in flight" (L3).
- `src/components/TrailsScreen.test.tsx`:
  - existing race test (L5) still passes.
  - "reads locally first, then refreshes from GitHub" (L2).
  - "keeps the page and spins Refresh during the GitHub read" (L3, L4).
  - "a superseded refresh triggers one catch-up read" (L6).
  - "shows Retry when the first read fails" (L7).
- `src/components/trails/TrailPeek.test.tsx`:
  - "opens on focus with the summary, then the latest events" (K3, K4).
  - "shows the last events at once on a second open" (K4).
  - "says the read failed and keeps the summary" (K5).
  - "Open trail calls onOpen" (K6).
- `src/components/BridgeSidebar.test.tsx` (or a new trail test file):
  - "shows a trail button only on chats on a trail" (K1).
  - "clicking the trail button opens the trail" (K2).
- `src/components/SessionToolbar.test.tsx`: the trail chip still opens the
  trail on click (K7).

## Integration / Functional Tests

- `bunx vitest run src/components/trails src/components/TrailsScreen.test.tsx src/components/BridgeSidebar*.test.tsx src/components/SessionToolbar.test.tsx src/designSystem.test.ts`
- `bun run build` (tsc -b + vite build) green.
- Full `bunx vitest run` green (no other file regresses).

## Smoke Tests

- Mock mode (`bunx vite --port 1421`): Apps → Trails lists trails; opening one
  paints; Refresh spins briefly; sidebar hover on a chat on a trail reveals the
  button; hovering it opens the card.

## E2E Tests

- Headless Chrome over CDP against mock mode: screenshots of the trail page
  (dark, light), the loading state (readTrail delayed), the sidebar trail
  button and the open card (dark, light), and the header chip card.

## Manual / cURL Tests

N/A: no wire or backend change. Real-app check: open a trail with PRs in the
desktop app; the page shows at once and the Refresh icon spins while `gh`
runs.
