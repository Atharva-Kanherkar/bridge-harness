# fix/streaming-smoothness — Test Contract

Two user-visible defects in the chat pane, plus the streaming polish the user
asked for, for every harness (Claude, Codex, OpenCode, ACP/Cursor/Grok).

## Root causes (established before implementation)

1. **Band above the composer.** `App.tsx` paints a 2rem overlay
   (`bg-gradient-to-t from-background`) at the bottom of the transcript. The
   dark vibrancy canvas is `color-mix(background 82%, transparent)` over native
   material (sampled `#18171a` in the screenshot), so an opaque fade to
   `--background` (`#111`) draws a dark stripe with a hard edge where the
   composer's own canvas resumes.
2. **Reply shown twice.** Replaying real `bridge exec --json` captures (Claude,
   Codex) and the four golden fixtures through `appendAgentEventBatch` →
   `reduceConversation` → `mergeConversationProjections` at every forest-lag
   cut point finds **no logical duplicate**. The duplicate is a render
   artifact: a single row is React-keyed by `item.key`, which is the provider
   item id (`msg_…`) on the live projection and `entry:<id>` on the durable one.
   When the 3 s forest poll lands, the merge keeps the durable twin, so under
   `AnimatePresence` the live row plays its exit fade while the durable row
   fades in: two copies of the reply on screen, then a jump. Groups already key
   off `identity` (`group:<identity>`); single rows do not.
3. **Streaming looks rough.** Prose streams at `opacity: 0.7` (`.md.dim`) and
   pops to full ink on completion; provider chunks arrive in bursts (a word
   fragment, then a sentence), so text lurches.

## Functional Behavior

- F1. No painted overlay sits between the transcript and the composer. The
  transcript's bottom edge fades via a CSS mask on the scroll container, which
  fades content to transparent and so matches any canvas: opaque, translucent
  vibrancy, light, dark.
- F2. A streamed reply keeps the same DOM node from its first token through
  the live→durable swap. At no point, including mid exit animation, are
  there two copies of one reply in the DOM.
- F3. Row keys stay unique: if two rows share an identity, the later one falls
  back to a unique key rather than colliding.
- F4. Streaming prose renders in full ink. There is no dim-to-full transition
  when a message completes.
- F5. Streaming assistant prose is revealed smoothly: the displayed length
  advances every animation frame toward the received length, at a rate
  proportional to the backlog, so a burst drains over a short, bounded window
  (target ~120 ms) instead of landing as one lurch.
  - Never shows text that has not been received.
  - Never lags more than the drain window: a big backlog reveals faster.
  - A settled (non-streaming) message renders its full text immediately.
  - A message that completes mid-reveal finishes revealing without losing text.
  - Reduced motion renders full text immediately.
  - Text that changes non-monotonically (replacement, not append) snaps to the
    new text.
- F6. Scroll-follow keeps a pinned reader at the bottom while the reveal
  drains.
- F7. Holds for every harness: Claude, Codex, OpenCode, ACP (Cursor).

## Unit Tests

- `src/transcript/grouping.test.ts`
  - `keys an item row by its cross-projection identity` — live and durable
    projections of the same golden stream give each assistant reply the same
    row key.
  - `falls back to a unique key when two rows share an identity`.
- `src/components/smoothText.test.ts` (pure reveal step)
  - never reveals past the target
  - drains a backlog within the window, at least one char per frame
  - bigger backlog → bigger step
  - non-prefix change snaps
- `src/components/AgentConversation.test.tsx` (jsdom)
  - `keeps one reply node through the live-to-durable swap` — render live
    streaming + completed events, then rerender adding the forest entries for
    the same stream: exactly one element contains the reply text, and it is the
    same DOM node as before.
  - `streams prose at full ink` — no `.md.dim` while streaming.

## Integration / Functional Tests

- Replay across all four golden fixtures (`claude`, `codex`, `opencode`,
  `cursor`): at every durable-prefix cut, the rendered row key of each
  assistant reply is constant and there is exactly one row per reply.

## Smoke Tests

- `bun run build` green.
- `bun run test` green (sidecar node:test + vitest + cargo test).

## E2E Tests

N/A — no browser E2E harness for the Tauri transcript. Covered by the jsdom
swap test and manual check below.

## Manual Tests

- `bun run dev` (mock data): send a message, watch the reply stream; it flows
  evenly, in full ink, and does not flash or duplicate when it settles.
- Look at the composer boundary in dark and light themes: no stripe, no hard
  edge; transcript prose fades out as it scrolls under the composer.
- Desktop app, dark vibrancy skin: same boundary check against the screenshot.
