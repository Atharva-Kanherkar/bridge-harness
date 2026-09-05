# feat/issue-487-harness-behavior-contract — Test Contract

The user-visible half of #488. #488 made the transcript's *inputs* uniform: one
closed event union, one pure reducer, no harness branches under
`src/transcript/`. This branch makes the transcript's *behavior* uniform and
writes it down: one thinking presentation, one set of grouping invariants, one
set of stream-state meanings, and a gate wide enough that a per-harness branch
cannot come back in through a component nobody listed.

Locked before implementation.

## Where the inconsistency actually lives

Enumerated, not guessed:

| Surface | Today | Why it diverges |
| --- | --- | --- |
| Cursor thoughts | live: two shimmering cards that never settle; replayed: none | `acp_events.rs` emits `reasoning.delta` only, and the forest refuses to persist a `.delta` |
| Codex / OpenCode / Claude thoughts | one card per thought, settled | all three emit a `reasoning.completed` |
| Streaming thought | `Brain` pulsing (`thinking-pulse`) + `Thinking…` sweeping (`text-shimmer-sweep`) | two animations for one state |
| Streaming reply with no text yet | a `thinking-shimmer` bar | a third animation for the same idea |
| Startup / model switch | `HarnessMark` turning + `text-shimmer` label | session-level, not item-level — legitimately its own thing |
| Harness branching | gate covers five transcript files | a rendering component outside that list is unpoliced |

## Functional Behavior

1. **A Cursor thought ends where it actually ended.** A run of ACP
   `agent_thought_chunk` updates is closed by the adapter: the first non-thought
   session update after the run, or the end of the turn, publishes a
   `reasoning.completed` carrying the run's accumulated text and the same item
   id its deltas carried. The completion is a persisted kind, so the forest
   stores it.

2. **Every thought run has an id.** ACP's `messageId` is optional. A run the
   agent did not name gets one minted for it, stamped onto the run's deltas as
   well as its completion, so the live card and its durable twin are one item.
   Two runs are two ids. A run the agent interrupted and resumed under the same
   `messageId` finishes as one thought, not as two cards whose texts overwrite
   each other.

3. **The other three normalizers already close their reasoning, and it is
   checked rather than assumed.** Codex closes on `item/completed` for a
   reasoning item, OpenCode when the part carries `time.end`/`status:
   completed`, the Claude sidecar by emitting `reasoning.completed` directly.

4. **Live and durable agree on Cursor.** With 1 and 2 in place, the golden
   live/durable parity assertion holds for all four harnesses with no
   per-harness exception, and divergence 11 leaves
   `testing/feat-issue-488-normalized-events.md`.

5. **One thinking presentation.** Transcript-level thinking is drawn by exactly
   one component, driven only by the normalized item's `status` — never by
   harness, never by wire kind. Two states: `streaming` (one achromatic
   animation, the `thinking-shimmer` sweep) and `completed` (collapsed by
   default, expandable). The same component draws a thought wherever a thought
   appears, including inside a group.

6. **No second thinking animation.** The `thinking-shimmer` sweep is the one
   mark for "the agent is thinking and nothing has landed yet". A streaming
   assistant reply with no text yet draws that same mark from that same
   component rather than an open-coded copy of it. `PulseDot`
   (`thinking-pulse`) means something else — *this item is in progress* — and
   is not a thinking indicator.

7. **Non-transcript indicators stay, and are named.** The startup narration row
   (`StartupStatusRow`, driven by session phase rather than by an item) is a
   pre-transcript indicator that unmounts as the first item starts streaming.
   It is documented in the behavior contract rather than folded into 5.

8. **A behavior contract exists and is written over normalized types.**
   `docs/transcript-behavior-contract.md` states thinking presentation,
   grouping and density *invariants* (not one algorithm — the algorithm is
   being rewritten on the #486 branch), stream-state semantics, and identity
   rules. No harness name appears as a condition anywhere in it. It is linked
   from `docs/session-forest.md` and from `AGENTS.md`.

9. **The gate covers every component.** Rule 1 of the harness branch gate (no
   `harness === "…"`) applies to every file under `src/components/**/*.tsx`
   plus `src/App.tsx`, with an explicit allowlist carrying a one-line reason per
   entry. Nothing in a transcript rendering path is allowlisted.

10. **Rendered parity is asserted across harnesses, not against a snapshot.**
    The golden render test compares a structural digest of the rendered DOM —
    row kinds, group count, thinking-state markers, collapsed/expanded state —
    between the four harnesses, so it keeps meaning when grouping changes
    underneath it.

11. **Mid-turn parity.** Truncated right after the first thought's delta and
    before its completion, all four harnesses render one identical streaming
    thinking presentation: same component, same shimmer marker, same state.

12. **Durable parity, rendered.** Each fixture's durable projection renders the
    same structure as its live reduction.

---

## Unit Tests

### `src-tauri/bridge-core/src/acp_events.rs`
- `closes a thought run before the tool call that ended it` — chunk, chunk,
  then a `tool_call` update: a `reasoning.completed` carrying both chunks'
  text and the run's item id is returned ahead of the tool event. (1)
- `closes a thought run left open at the end of a turn` — chunks, then the turn
  ends: the completion is emitted. (1)
- `gives two runs two ids` — chunks, a tool call, more chunks: two completions,
  two distinct item ids, each carrying only its own run's text. (2)
- `stamps the run's id onto the deltas the agent did not name`. (2)
- `finishes a resumed thought as one thought` — same agent `messageId` either
  side of a tool call: the second completion carries the whole thought. (2)
- `keeps a closed run closed` — a second non-thought update after a run has
  closed emits nothing. (1)

### `src-tauri/bridge-core/src/agent.rs` (existing, must stay green)
- `normalizes_codex_reasoning_deltas_with_stable_item_id` and the OpenCode and
  Claude reasoning cases: the check for behavior 3. No change expected.

### `src/transcript/golden.test.ts`
- `normalizes every cursor frame into the union, never into unknown` —
  `EXPECTED_EVENT_TYPES.cursor` now carries `thinking.completed` after each
  `thinking.delta`, matching what the adapter emits. (1)
- `agrees between the live and durable projections of the %s turn` — the
  `harness === "cursor"` exception is deleted; all four take the same path. (4)

### `src/transcript/golden.render.test.tsx`
- `draws the same transcript for every harness` — a structural digest
  (row kinds, group count, thinking states, open/closed) compared across the
  four, not against a literal. (10)
- `draws one streaming thought mid-turn, whoever is thinking` — each fixture
  truncated after its first thought's delta; all four digests equal, and the
  digest names a streaming thinking row with the shimmer marker. (11)
- `draws the durable projection the same way it drew the live one`. (12)

### `src/transcript/harnessBranchGate.test.ts`
- `no component branches on harness identity` — over every
  `src/components/**/*.tsx` and `src/App.tsx`, minus a named allowlist. (9)
- `the allowlist explains itself` — every allowlisted path exists, still
  matches the branch pattern it was allowlisted for, and carries a reason. An
  allowlist entry that stops being needed fails the test. (9)
- `one component owns the thinking presentation` — `AgentConversation.tsx`
  contains exactly one `thinking-shimmer` call site, and it is inside the
  thinking component. (5, 6)

### `src/components/AgentConversation.test.tsx` / `.transcript.test.tsx` (existing)
- All cases stay green; `Thought for a moment`, the collapsed-by-default
  disclosure, and the streaming card keep their observable behavior.

---

## Integration / Functional Tests
- The four golden fixtures, reduced and rendered, live and durable (above).
- `cargo test -p bridge-core acp_events::` and `agent::` for the normalizers.

---

## Smoke Tests
- `bun run build` — `tsc -b && vite build`.
- `bunx vitest run` — the whole frontend suite.
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core acp_events::`
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core agent::`

---

## Out of scope, deliberately

- `groupItems`, `ActivityGroup` and `ScrollFollow` in `AgentConversation.tsx`,
  and the reducer's grouping logic: the #486 branch owns them. This branch
  states grouping *invariants* in the behavior contract and asserts
  cross-harness equality, which is exactly the assertion that survives a
  grouping rewrite.
- Typing `ConversationItem.data`: divergence 1 of the #488 contract, unchanged.
- The session-level startup narration's own animation: behavior 7 documents it
  rather than merging it.
