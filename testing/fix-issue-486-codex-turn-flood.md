# fix/issue-486-codex-turn-flood — Test Contract

A hundred-step turn must read as one line of work, not as two hundred rows.
Locked before implementation. Builds on
[`feat-issue-488-normalized-events.md`](feat-issue-488-normalized-events.md):
the closed event union, the one codec, the one reducer, and
`ConversationItem.tool` are the base, not part of this change.

## What is actually wrong

Four faults compound, and only the first is visible:

1. **Grouping.** The emitter behind the item prefix opens a fresh reasoning
   item between every tool item (`src-tauri/bridge-core/src/agent.rs`,
   `normalize_item`). `groupItems` flushes the open activity group on every
   reasoning item, so a 100-step turn draws 100 one-item groups plus 100
   reasoning rows. `ecb0cfa8` had buffers that survived this; `5807c919`
   removed them.
2. **Expansion.** `ActivityGroup` latches `heldOpen` the moment it is ever
   live, so every group a reader watched stays open forever — 100 open cards.
3. **Plumbing.** Roughly four durable frames per step, each persisted
   (`live_turn.rs`) and each emitted individually over Tauri IPC
   (`src-tauri/src/lib.rs`, both the embedded forwarder and the daemon-client
   supervisor). The 50 ms JS flush in `App.tsx` is the only batching, and each
   flush hands the transcript a fresh `agentEvents` array.
4. **No memoization.** Every flush re-reduces, re-groups and re-reconciles
   every row, twenty times a second, because the reducer rebuilds every item
   object on every run.

## Functional Behavior

1. **Turn index on every item.** `reduceTranscript` stamps
   `ConversationItem.turn`, a 1-based index of the turn the item was created
   in (0 before the first boundary). Boundaries are derived from what **both**
   projections see, because `turn.*` frames carry `sequence: 0` and the
   durable writer refuses to persist them:
   - a `message.completed` with `role === "user"` opens a new turn, and the
     user's own row belongs to the turn it opens;
   - a `turn.started` opens a new turn **only** when no user message has
     opened one since the last boundary, so live and durable never differ by
     the boundary the live stream can see twice;
   - the pairing is order-free, because the two frames arrive in either order:
     a user message that lands in a turn no row has been created in yet joins
     that turn instead of opening another, and `turn.completed` releases the
     pairing so the next marker counts. Whichever frame opens a turn, it is
     counted once.

   *Documented divergence:* a turn started with no user message and no
   persisted frame of its own (an auto-continuation) is invisible to the
   durable projection, which therefore keeps the previous index. Nothing in
   the forest records that boundary; inventing one would be a guess.

   **Locked by:** `src/transcript/reducer.test.ts` — a two-turn live stream
   stamps 1 and 2; a `turn.started` after a user message does not double-count;
   a `turn.started` with no user message before it does count; and the four
   orders of a two-turn stream — marker before the prompt, prompt before the
   marker, a durable branch of prompts with no markers, and markers with no
   prompt at all — each stamp 1 and 2.
   `src/transcript/golden.test.ts` — the existing live-versus-durable parity
   assertion gains `turn` in its projected row, for all four harnesses.

2. **One tool group per run, thoughts inside it.** `groupItems` walks the
   items of one turn and keeps at most one open group:
   - a tool item (`activity`, `diff`, `artifact`) joins the open group, or
     opens one;
   - a `reasoning` or `plan` item is **held**: if another tool item follows in
     the same turn it is moved, in order, into the group's timeline; if the
     turn's tool work is over it is flushed as a top-level row;
   - every other item type (prose, approval, permission, question, error,
     delegation, checkpoint) flushes the held rows and closes the group, then
     renders top-level — narrative order is never rearranged;
   - a change of `item.turn` closes the group.

   So a thought before the first call stays a top-level row, a thought between
   two calls lives inside the group, and a thought after the last call and
   before the reply stays a top-level row. Top-level rows per turn are O(1) in
   the number of steps.

   *One numbering first.* The transcript groups a **merged** list, and the two
   projections that produced it each counted turns from their own start — the
   forest from the first message of the session, the live window from wherever
   it opened. Mid-turn the durable half of a run therefore carries one index
   and the live half another, the walk cuts the run at that seam, and every
   forest poll moves the cut. `alignTurns` re-stamps the merged list from the
   one boundary both projections can see — a user message opens a turn and
   belongs to the turn it opens, rows ahead of the first stay turn 0 — and runs
   in `AgentConversation`'s `useMemo` after the merge and the delegation fold,
   immediately before `groupItems`. Rows whose index does not change are
   returned unchanged, so the re-stamp invalidates no memo signature it did not
   have to. The reducer keeps its own stamp: it is the right answer for a
   single projection, it can see a marker-only boundary no user message
   separates, and the golden parity assertion is written against it.

   **Locked by:** `src/components/AgentConversation.flood.test.tsx` (jsdom) —
   the 100-step fixture draws exactly five top-level rows (user bubble,
   opening thought, one group, closing thought, reply) and the group's summary
   reads `Ran 62 commands, read 30 files, edited 8 files`.
   `src/transcript/grouping.test.ts` — the walk itself, case by case:
   thought inside a run folds in, thought after the run does not, a plan update
   does not split a run, prose does, a new turn does.
   `src/transcript/turnSeam.test.ts` — a two-turn stream whose forest holds the
   first half and whose live window opened with the second turn: without
   `alignTurns` the merged list draws more than one group, with it exactly one,
   the top-level shape is the same at 10 steps and at 40, and an already
   correct row comes back as the same object.

   *The fixture is a recipe, not a file.* The issue asks for
   `fixtures/codex-flood.json`; four hundred hand-written frames would be
   unreviewable and what is under test is a shape, so
   `src/transcript/fixtures/codexFlood.ts` generates the same stream — the
   frame-for-frame shape of `codex.json`, at any step count — and
   `fixtures/README.md` documents it beside the four golden streams.

3. **Collapsed by default.** The `heldOpen` latch is gone. A group is
   collapsed unless the reader expanded it; a live group shows the step
   running right now as a single line under the summary, and nothing else.
   The toggle is keyed on the group's identity (`group:<first item identity>`),
   which is stable across re-renders, across the live-to-durable merge, and
   across the live-to-complete transition, so a group the reader opened stays
   open and a group they closed stays closed.

   *Bounded exception, kept deliberately:* a run of at most three tool calls
   that carries a patch still opens itself. The repo's inline-diff doctrine —
   "what the model wrote is the most important thing on the screen", locked by
   four cases in `AgentConversation.transcript.test.tsx` — is not worth losing
   to fix a flood, and the bound is what makes it safe: past three calls the
   run is exactly the thing that must not open itself. `SELF_OPENING_STEPS`
   names it.

   Summary copy names what ran, commands first, no em dashes:
   `Ran 62 commands, read 30 files, edited 8 files` when complete,
   `Running 62 commands, reading 30 files…` while live. The step count counts
   tool items only, never the folded thoughts.

   **Locked by:** `AgentConversation.flood.test.tsx` — a completed group
   renders no `ActionRow`s until its summary button is clicked, and the
   toggle survives a re-render that changes an unrelated item.

4. **Nothing recomputes without new input.** `reduceConversation`,
   `projectSessionConversation`, `mergeConversationProjections` and
   `groupItems` run inside `useMemo` with minimal deps, so a render with
   referentially unchanged `events`, `forestEntries` and `activeLeafId` runs
   none of them. `ActivityGroup`, the message row and the reasoning row are
   `React.memo` with a comparator over a cheap item signature
   (`identity`, `status`, `text.length`, `title`, `eventId`, `sequence`, and
   the tool facet's status), because the reducer rebuilds every item object on
   every run and reference equality is therefore worthless.

   **Locked by:** `src/components/AgentConversation.memo.test.tsx` — a flush
   that appends output to one tool call re-renders that row and leaves the
   other rows' render counts unchanged.

5. **One IPC message per flush window.** The Tauri shell batches live agent
   events at the emit boundary: `agent-event` payloads accumulate for at most
   16 ms (or 64 events) and are delivered to the webview as a single
   `agent-event-batch` carrying `AgentEvent[]`. Any other event flushes the
   pending batch first, so relative order is preserved. Persistence is
   untouched — the batcher sits after the store, on the way to the webview
   only, and both host paths (embedded forwarder and daemon-client supervisor)
   go through it. `bridgeApi.onAgentEvent` unpacks the batch and calls its
   handler once per event, so `App.tsx` and every other subscriber are
   unchanged.

   **Locked by:** `src-tauri/src/agent_batch.rs` unit tests — events under the
   window accumulate; the size cap flushes early; a foreign event flushes the
   pending batch ahead of itself; the deadline flushes what is held.

6. **Bounded and quick under a flood.** For the 100-step fixture
   (`src/transcript/fixtures/codex-flood.json`), top-level rows stay bounded
   and `reduceTranscript` followed by `groupItems` completes well inside a
   generous budget on the test runner.

   **Locked by:** `src/transcript/flood.perf.test.ts` — five top-level rows at
   10, 100, 400 and 1000 steps; every call accounted for in the one group; the
   median of five reduce-plus-group passes under 50 ms; and 400 steps costing
   under ten times what 100 did. The thresholds are generous on purpose: this
   is a guard against a return to per-step structure, not a benchmark, and a
   CI runner under load must not fail it. Measured on this machine: 0.77 ms
   for 100 steps (468 frames), 1.57 ms for 400, 3.70 ms for 1000 — linear.

   `src/agentEventBatch.test.ts` closes the other half of the flush path: one
   batched IPC message is delivered to subscribers one frame at a time, in
   order.

7. **Windowing of collapsed history** for 1000+ top-level rows is **out of
   scope** for this branch and is not implemented. With behaviors 2 and 3 in
   place a flooded turn contributes a handful of top-level rows rather than
   two hundred, which is the reported fault; windowing addresses long
   *sessions*, is a separate change, and collides with the `ScrollFollow`
   rewrite on another branch.

## Scope boundaries

- `ScrollFollow` and the scroll container props are untouched (issue 485).
- `harnessBranchGate.test.ts` and the `Reasoning` component's visuals are
  untouched (issue 487). Thoughts inside a group render through the existing
  `Reasoning` component, unmodified.
- `src/transcript/codec.ts` is untouched. The turn stamp is a reducer change.
- **One necessary edit inside a sibling's file:**
  `src/transcript/golden.render.test.tsx` asserts the old grouping for the
  golden turn (`user-bubble, thought, activity-group, thought, activity-group,
  assistant-prose`) with a comment that spells out the very regression this
  branch fixes — "the command stands alone because a thought interrupts it".
  Behavior 2 makes that turn draw `user-bubble, thought, activity-group,
  assistant-prose`. Only the expected array and its explanatory comment
  change; the file's structure, helpers and other cases are left alone.
- Three other existing test files move with the behavior they describe:
  `AgentConversation.transcript.test.tsx` (the two plan cases now assert one
  run with the plan inside it, per behavior 2),
  `AgentConversation.motion.test.tsx` (the glyph case opens the finished run
  by hand rather than relying on the removed latch), and
  `api.boundary.test.ts` (the batched stream joins the menu channel on the
  named list of shell-owned Tauri events).

## Non-goals

- No change to what is persisted, or to the durable writer's rules.
- No new thinking presentation, and no second thinking component.
- No harness-string branch anywhere in `AgentConversation.tsx`.
