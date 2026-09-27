# feat/transcript-attention-and-thinking-toggle — test contract

A thinking toggle in Appearance, a failed run that names who must act, no
exit-code chips, a settled transcript that stops claiming live work, and
OpenCode replies that survive a reload.

## Thinking preference — `src/transcriptSettings.test.ts`

- [x] Missing storage means thinking is shown; a stored `"false"` means it is hidden.
- [x] Writes round-trip, and a denied storage reads as the default instead of throwing.

## Transcript — `src/components/AgentConversation.transcript.test.tsx`

- [x] Thinking on (default): a settled thought draws its collapsed `details` and its text.
- [x] Thinking off: a streaming thought keeps the pulsing `[data-thinking-row]` and drops its text.
- [x] Thinking off: a settled thought draws no row at all, so it leaves no empty wrapper or gap.
- [x] A command's exit code is never printed, for zero, nonzero, or an unreported code.
- [x] A failed run is marked `needs the agent`; the transcript never claims the reader's attention.
- [x] A settled failed run opens on its failed rows only, and "Show all N steps" reveals the rest.
- [x] A healthy run still opens whole.
- [x] A live check reads `Running` while the turn is active, and `Pending` with no
      `Working` header once the turn is over — the item keeps its wire status,
      the presentation stops claiming it.

## Settings — `src/components/settings/AppearancePage.test.tsx`

- [x] `Show thinking` is on by default and persists its new value to local storage.

## OpenCode durability — `src-tauri/bridge-core/src/agent.rs` (agent::tests)

- [x] Prose that only ever arrived as deltas is assembled into one
      `message.completed` at turn end, carrying the part's full text.
- [x] A finished text-part snapshot supersedes its delta run: exactly one
      message, no flush duplicate at idle.
- [x] Subagent prose is never flushed as the root chat's reply.

## Docs

- [x] `docs/transcript-behavior-contract.md` states that hiding thinking is a preference, not a third state.
- [x] It states that liveness is scoped to the turn even though item statuses are not.
