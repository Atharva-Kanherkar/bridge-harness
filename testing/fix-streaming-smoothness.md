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
3. **Thought shown twice.** A *logical* duplicate, and the one defect above does
   not cover. Nearly every harness sends `reasoning.*` with **no provider item
   id**, so `liveKey` returns no key (the reducer borrows the turn's) and the
   durable writer files the same frame under the forest entry's own id. The live
   row's identity is `reasoning:<event id>`; its persisted twin's is
   `entry:<entry id>`. Two numbering spaces that can never agree, and
   `mergeConversationProjections` deduplicates on identity — so it kept both and
   `coalesceThoughts` joined them into a single Thinking card whose body was the
   same paragraph twice. `assistantShadowText` already matched unnamed assistant
   prose on text for exactly this reason; a thought never got the same answer,
   and is the commoner case. Mid-stream it is worse than a duplicate: a delta is
   never persisted, so the live row holds only the *prefix* of a body the forest
   already holds whole, and they cannot even be compared for equality until the
   stream lands.
   - All four golden fixtures happen to **name** their reasoning
     (`reasoning-msg-1`, `thought-2`, …), so none of them can see this. The
     sweep strips the names.
   - The sweep must move **both** axes. The live window is a *tail* of the
     stream, not the whole turn. Holding the live window at the full turn and
     sweeping only the forest hides the defect completely: the live window then
     keeps only the turn's last thought while the forest holds each one
     separately, so no two rows ever say the same thing. The doubled card needs
     the live window to still be holding the thought the forest has just
     stored, which is what a live tail is.
4. **Streaming looks rough.** Prose streams at `opacity: 0.7` (`.md.dim`) and
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
  - **Settling is watched as well as growth.** A terminal frame routinely carries
    no new characters, only `streaming: true -> false`, so a hook that recalculates
    only on a text change leaves a reply on a truncated prefix for the rest of the
    window while its own action bar is already showing. No frames advanced: a
    settled row has nothing to drain.
  - Reduced motion renders full text immediately.
  - Text that changes non-monotonically (replacement, not append) snaps to the
    new text.
- F6. Scroll-follow keeps a pinned reader at the bottom while the reveal
  drains.
- F7. Holds for every harness: Claude, Codex, OpenCode, ACP (Cursor).
- F8. Thinking is never suppressed. A thought is still drawn by the one thinking
  component, still streams with the shimmer and still settles to a collapsed
  summary, it is drawn **once**. Reconciling the two projections must not
  remove, collapse or hide a thought to achieve that.
- F9. A row the merge cannot match by identity is matched on its whole trimmed
  body instead. A row it drops that way is replaced by one that says exactly the
  same thing, and the durable row is the survivor, it is the one the reader can
  branch from.
- F10. The prefix match that covers the mid-stream case is scoped and floored,
  so it cannot cost a real thought: it only ever compares against the **newest**
  stored thought (the thought being streamed is the newest thing in the forest),
  and only when at least 24 characters have arrived. Below that floor a
  half-streamed thought is kept, because the opening words of two thoughts are
  the part they are most likely to share.
- F11. **Deduplicating a row is not the same as replacing it.** A survivor the
  merge paired on text has to answer to the identity the reader was already
  looking at, or `rowKey` gives it a different key and `AnimatePresence` plays
  the old row's exit while the new one enters. That is the doubled row again, one
  layer up, and it applies to every text-matched pairing: the unnamed reply the
  review found, and the thought paired by prefix.
  - The row keeps its own durable identity for everything else. Only what the
    reader is looking at is borrowed.
  - The DOM node still does not survive the swap for a thought, and is not meant
    to: the streaming state is a card and the settled state a collapsed
    `details`, one component with two shapes. What must not happen is both on
    screen at once.

## Unit Tests

- `src/transcript/grouping.test.ts`
  - `keys an item row by its cross-projection identity` — live and durable
    projections of the same golden stream give each assistant reply the same
    row key.
  - `falls back to a unique key when two rows share an identity`.
- `src/conversation.test.ts` (the merge, F8–F11)
  - `draws a thought once once the forest has caught up with it` — asserts the
    two identities genuinely differ first, so the test cannot be satisfied by
    keying on identity.
  - `does not double a thought the forest caught up with mid-stream` — the live
    row is a prefix of the stored one.
  - `reconciles a named thought on its item id alone` — Codex names its
    reasoning; identity alone suffices.
  - `keeps a new thought that no durable row says`
  - `keeps a new thought that only opens like the one before it` — F10 scope.
  - `keeps a streaming thought too short to be the stored one` — F10 floor.
  - `keeps a thought on the identity the reader was already watching`: F11,
    mid-thought, the pairing no text equality can make.
  - `keeps a settled thought on the identity it streamed under`: F11, and the
    survivor is still the stored row.
  - `keeps an unnamed reply on the identity it streamed under`: F11 on the path
    the review opened.
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
  - `keeps one reply node when the swap runs on the unnamed text match`: the
    review's case, only an unnamed live `message.delta`, then its text-matched
    stored entry with no terminal live event. Finds **2** nodes on the unfixed
    merge and **1**, the same node, after.
  - `streams prose at full ink` — no `.md.dim` while streaming.
- `src/transcript/golden.test.ts` (real captured turns, names stripped, F8–F10)
  - `never shows a <harness> thought twice, at any live and durable cut` — every
    `(liveCut, forestCut)` pair across all four fixtures. Fails on the unfixed
    merge at `live 2, forest 2` with the card body
    `"Start with the suite.\nStart with the suite."`.
- `src/components/smoothText.hook.test.tsx` (jsdom, F5 settle)
  - `shows the whole reply at once when only the status settles`: the same text
    with `streaming` flipped false, asserted before a frame is advanced.
- `src/components/AgentConversation.transcript.test.tsx` (jsdom, F8, F11)
  - `draws a thought once when the forest catches up with it` — one
    `[data-thinking]` card, `data-thinking="completed"`, summary still reads
    "Thought for a moment", and the body holds the thought once.
  - `keeps one thought card on screen when the body it streamed arrives`: one
    card across the mid-thought seam. Two on the unfixed pairing: the streaming
    one exiting and the settled one entering.

## Integration / Functional Tests

- Replay across all four golden fixtures (`claude`, `codex`, `opencode`,
  `cursor`): at every durable-prefix cut, the rendered row key of each
  assistant reply is constant and there is exactly one row per reply.
- The two-axis sweep above is that replay for reasoning, with the names removed.

## Known and out of scope

- An unnamed provider gets **one thought per turn**: `liveKey` hands an unnamed
  reasoning frame the turn's key on purpose (`codec.ts`, "Reasoning and prose
  without an item id borrow the turn's key"). So a live window that has already
  seen two thoughts keeps only the second, while the forest still holds the
  first, and `coalesceThoughts` puts both in one card in reverse order. Real, and
  visible, but a different defect from a doubled thought, and fixing it would
  change what unnamed harnesses show rather than stop them showing it twice.
  Left alone here on purpose.

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
