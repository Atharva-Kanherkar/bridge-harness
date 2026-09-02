# feat/issue-452-cross-harness-ui-parity — Test Contract

## Functional Behavior

1. **Codex Thinking / Reasoning Normalization & Coalescing**
   - Codex emits `item/reasoning/textDelta` and `item/reasoning/summaryTextDelta`. Normalization in `src-tauri/bridge-core/src/agent.rs` ensures a stable `item_id` is assigned (derived from active reasoning item or session state, e.g. `reasoning-1`) rather than leaving `item_id` empty.
   - `item/reasoning/summaryPartAdded` is handled and normalized into reasoning content rather than discarded as an internal notification.
   - In `src/agentEvents.ts`, `mergeKey` falls back to a session-bound reasoning key for `reasoning.delta` when `itemId` is absent, preventing event flooding and layout thrashing.
   - In `src/conversation.ts`, `reduceConversation` resolves streaming reasoning items when `item/completed` (`reasoning.completed`) arrives even if text is empty or keys have minor mismatches, and settles any active streaming reasoning upon turn completion so the pulsing `Thinking…` indicator is never stuck indefinitely.

2. **OpenCode Thinking Normalization & Completion**
   - In `src-tauri/bridge-core/src/agent.rs`, `normalize_opencode_message_with_state` does not drop reasoning deltas when `state.message_roles` has not yet received `message.updated` (defaults to assistant for reasoning deltas).
   - `normalize_opencode_part` checks both `part.reasoning` and `part.text`.
   - `reasoning.completed` is emitted upon reasoning completion (even if `/time/end` is omitted or delayed, checking part status or turn idle).

3. **Forest Projection Parity on Reload (All Harnesses)**
   - In `src/conversation.ts` (`projectSessionEntry`), durable forest entries with `entry.kind === "reasoning.completed"` or `entry.kind === "reasoning"` (or starting with `reasoning.`) project to `type: "reasoning"`, `status: "completed"`, `title: "Thought for a moment"`.
   - On page reload or session switch, historical reasoning renders as collapsible `<Reasoning>` ("Thought for a moment") across all harnesses, never as a tool wrench activity card.

4. **Exploratory Command Classification & Hairline Folding**
   - In `src/conversation.ts` (`toolCallDisplay`), read-only CLI commands (`cat`, `head`, `tail`, `less`, `more`, `bat`, `ls`, `dir`, `tree`, `grep`, `rg`, `ag`, `ack`, `find`, `fd`, `which`, `whereis`, `wc`, `git status`, `git diff`, `git log`, `git show`, `git branch`) are classified as `verb: "read"` or `verb: "search"`.
   - Because `read` and `search` belong to `FLAT_VERBS`, exploratory commands fold into the compact hairline rows under `<GroupLabel>Explored</GroupLabel>`, while retaining clickable path links to the Code pane when a file argument is present.
   - Mutating commands (`git commit`, `git push`, `rm`, `cargo build`, `bun test`, etc.) remain `verb: "run"` and render as full terminal cards with prompts and exit chips.

5. **Turn-Aware Activity & Thought Coalescing**
   - In `src/conversation.ts` and `src/components/AgentConversation.tsx`, interleaved plan updates (`turn/plan/updated`) and reasoning do not shatter consecutive tool calls in an assistant turn into multiple 1-item `ActivityGroup`s.
   - All distinct plan items are preserved in arrival sequence without dropping or overwriting.
   - Tool calls, diffs, and exploratory commands coalesce into a unified `ActivityGroup` summarizing what ran (e.g. `Ran commands, read files`) with a green checkmark upon completion.
   - Thoughts occurring after commands preserve their natural post-execution position rather than being reordered above commands.
   - In `ActivityGroup`, user toggle state (`toggled`) takes precedence over `live`, allowing the user to collapse live groups, and completed groups collapse cleanly.

6. **Error Handling & State Hardening**
   - Codex `turn/completed` with `status: "failed"` or `"error"` synthesizes a conversation `error` event, rendering an actionable `ErrorCard` in chat.
   - `process/exited` represents spawned command/exec sessions rather than the host binary, so it remains in `is_codex_internal_notification` to prevent failing sessions on tool command exit codes.
   - Codex errors with `willRetry: true` or fatal errors do not leave the session permanently in a "working" or hung state.
   - OpenCode stream disconnects or errors reliably surface visible error notices.

---

## Unit Tests

### Rust (`bridge-core::agent`)
- `tests::normalizes_codex_reasoning_deltas_with_stable_item_id` — verifies Codex text deltas carry stable, incrementing per-turn `item_id`.
- `tests::normalizes_codex_summary_part_added_into_reasoning` — verifies summary part is not dropped.
- `tests::normalizes_codex_turn_completed_with_failure_synthesizes_error` — verifies failed/error turn produces error event.
- `tests::normalizes_opencode_reasoning_deltas_before_message_updated` — verifies early reasoning deltas stream through without misattributing non-reasoning parts.
- `tests::normalizes_opencode_reasoning_streaming_midstream_snapshot` — verifies midstream reasoning snapshots default to inProgress delta.
- `tests::normalizes_opencode_reasoning_field_variants_and_completion` — verifies `part.reasoning` is read when `part.text` is absent and completes on finish.

### TypeScript (`src/`)
- `src/agentEvents.test.ts`:
  - `coalesces reasoning.delta even when itemId is absent by using fallback key`
  - `clears reasoning merge indexes on turn completion`
- `src/conversation.test.ts`:
  - `projects forest reasoning entries to type: "reasoning" with completed status`
  - `reduces Codex reasoning deltas and transitions to completed upon item/completed`
  - `settles streaming reasoning to completed on turn.completed`
  - `keeps separate reasoning cards across turns when itemId is null`
  - `classifies read-only commands (cat, ls, grep, git status, git diff) into read/search verbs`
  - `keeps mutating commands (git commit, bun test, rm, redirects) as run verb`
  - `handles pipe and && chains in exploratory commands`
  - `handles quoted arguments with spaces and harmless arrow patterns`
- `src/components/AgentConversation.transcript.test.tsx`:
  - `renders replayed reasoning as collapsible Thought for a moment details block`
  - `coalesces tool calls across interleaved plan updates into a single ActivityGroup`
  - `preserves multiple distinct plan items without dropping`
  - `preserves reasoning order when a thought occurs after commands`
  - `folds exploratory CLI commands into Explored hairline section`

---

## Integration / Functional Tests
- Full conversation reduction test with simulated Codex turn: reasoning deltas -> exploratory commands -> plan updates -> file edits -> turn completed.
- Full conversation reduction test with simulated OpenCode turn: assistant reasoning -> tool calls -> completed turn.
- Session reload test: active branch with historical reasoning, diffs, and exploratory commands projects to proper components.

---

## Smoke Tests
- `bun run check`: `tsc -b --pretty false && cargo check --manifest-path src-tauri/Cargo.toml --workspace`
- `bun run test`: `npm test --prefix sidecar/claude-agent && vitest run && cargo test --manifest-path src-tauri/Cargo.toml --workspace`
- `bun run build`: `tsc -b && vite build`

---

## E2E Tests
N/A — Desktop app runs against mock data in component tests and headless harnesses in Vitest.

---

## Manual / cURL Tests
1. Start dev frontend: `bun run dev`
2. Open transcript with Codex session containing reasoning and shell commands.
3. Observe pulsing `<Brain /> Thinking…` during reasoning stream, settling to `<details> Thought for a moment`.
4. Observe exploratory commands (`cat`, `ls`, `git status`) listed under "Explored" hairline section.
5. Observe mutating commands (`bun test`, `git commit`) displayed as terminal cards.
6. Observe multiple consecutive tool calls grouped into a single summary button ("Ran commands, read files").
7. Refresh page (reload) and verify historical reasoning still displays as "Thought for a moment", not a wrench tool card.
