# fix/issue-485-open-at-latest — Test Contract

Field report: opening an old chat lands the viewport at the top, on the first
message. Reaching the most recent message means scrolling the whole transcript
by hand. It should land on the latest message, or on the last-read position.

Root cause (read from the code, `origin/main`): there is no scroll-restore or
open-at-latest logic. The only mechanism is `ScrollFollow`
(`src/components/AgentConversation.tsx`), which scrolls to the bottom only when
its `signature` changes *and* `pinned` is still true. Four gaps undermine it:

1. **The transcript loads in stages, so the top paints first.** History arrives
   from the forest snapshot asynchronously (`src/App.tsx` forest poll); the
   in-memory cache is empty for a chat not opened this app run. The sequence is
   greeting or empty state (no `ScrollFollow` mounted) → forest arrives → the
   whole transcript commits in one render → `ScrollFollow` mounts fresh at
   `scrollTop = 0`. The first painted frame *is* the top.
2. **The scroll's own animation disarms the follow flag.** The container carries
   `scroll-smooth`, so the post-paint correction is an animated scroll. While it
   is in flight `onScroll` fires with the viewport far from the bottom, flipping
   `pinned` to false mid-scroll; later signature changes then no-op.
3. **Nothing resets on session switch.** `AgentConversation` mounts without a
   key, `ScrollFollow` sits outside the session-keyed `AnimatePresence`, and the
   signature carries no session id, so `pinned` and `scrollTop` bleed across
   chats.
4. **`useEffect` runs after paint,** so even the happy path shows one frame of
   the top before correcting.

This change is frontend-only and contained to `ScrollFollow` and the container
it renders. No backend, no protocol, no reducer or grouping changes.

## Functional Behavior

1. **Open at the latest message, in the first painted frame.** Placement runs in
   a layout effect (before paint), and every programmatic scroll is instant
   (`scrollTo({ behavior: "instant" })`, with a `scrollTop` assignment as the
   fallback when `scrollTo` is unavailable). Opening a chat never shows a frame
   of the top and never glides down from it.
2. **Late history still lands.** Placement is keyed to the *first population* of
   a session, not to mount. If `ScrollFollow` mounts with an empty transcript
   (a working shimmer, a notice) and the forest snapshot arrives seconds later,
   the landing happens on that first populated commit.
3. **Session switch resets, and switching back restores.** The placement,
   `pinned`, and last-read state are keyed by session id. Switching to another
   chat and back restores that chat's last-read `scrollTop`; a chat that was
   left at the bottom (`pinned`) reopens at the bottom, which is where any newer
   message is. A chat with no record opens at the bottom.
4. **Programmatic scrolls are not user scrolls.** Each programmatic scroll
   records the target it just set; the `scroll` event it produces is matched
   against that target and does not recompute `pinned`, so live follow cannot
   disarm itself. A genuine user scroll (a position that does not match the
   recorded target) still sets `pinned` from the distance to the bottom, with
   the existing 80px threshold.
5. **Live follow still sticks.** While `pinned`, each signature change (new
   item, streaming text growth, an optimistic bubble, the working flag) re-pins
   to the bottom. Content that changes height after commit (async syntax
   highlighting, images) re-pins through a `ResizeObserver` while pinned, guarded
   for environments that lack one.
6. **User intent wins.** A reader who scrolls up mid-stream stays where they
   are; their position is recorded for that session as they scroll.
7. No em dashes in any user-visible string; no user-visible copy changes at all.

## Unit Tests

`src/components/AgentConversation.test.tsx` (extended, jsdom — `act` +
`react-dom/client`, per the file's existing helpers). jsdom has no layout, so
`scrollHeight`/`clientHeight` are stubbed on `HTMLElement.prototype` and
`scrollTo` is installed on the prototype (jsdom leaves it undefined), recording
`{ top, behavior }` and assigning `scrollTop`.

- `lands on the latest message when history arrives after mount` — mount with
  no forest entries, then supply a long transcript through `act`; asserts
  `scrollTop === scrollHeight - clientHeight` (behavior 1 and 2).
- `lands instantly, never gliding down from the top` — asserts every recorded
  programmatic scroll used `behavior: "instant"` and that no scroll landed at
  the top (behavior 1).
- `does not treat its own scroll as the reader leaving the bottom` — after the
  landing, dispatches the `scroll` event the programmatic scroll would produce,
  then grows the transcript; asserts it still follows to the bottom, i.e.
  `pinned` was not flipped (behavior 4).
- `treats a real scroll away from the bottom as intent and stops following` —
  sets `scrollTop` to the top, dispatches `scroll`, grows the transcript;
  asserts the viewport stays put (behaviors 4 and 6).
- `restores the last-read position when a chat is reopened` — scrolls a session
  to a mid position, switches the `session` prop to another chat, switches back;
  asserts `scrollTop` returns to the recorded position rather than the bottom
  (behavior 3).
- `opens a chat with no recorded position at the bottom` — switching to a
  session never opened lands at the bottom (behavior 3).
- Every existing case in the file stays green, including the
  `renderToStaticMarkup` cases that render the transcript on the server.

## Integration / Smoke

- `bun run build` and `bunx vitest run` green.
- Mock mode (`bun run dev`): open a chat with a long transcript, confirm it
  opens at the last message with no visible top frame; scroll up, switch chats,
  switch back, confirm the position is where it was left.

## E2E

N/A — desktop app.

## Manual Tests (reviewer)

1. Open a long, previously unopened chat from a cold app start (empty forest
   cache): it lands at the last message, with no glide and no top flash.
2. Scroll up in that chat, switch to another chat, switch back: the position is
   restored.
3. Send a message and let it stream: the transcript follows the bottom for the
   whole turn.
4. Scroll up mid-stream: the transcript stops following and stays where you put
   it; scrolling back to the bottom resumes following.
