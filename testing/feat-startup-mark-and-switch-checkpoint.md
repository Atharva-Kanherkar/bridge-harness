# feat/startup-mark-and-switch-checkpoint — Test Contract

Two related complaints about the cold-start experience shipped in #360:

1. The startup row is a label plus a grey bar. It says nothing about *which* agent
   is starting, and it has no life in it.
2. Switching the model on a chat that had only exchanged "hi" produced a wall of
   agent prose in the transcript — the outgoing model refusing to fabricate a
   handoff checkpoint — followed by a `Compaction requested / before_downgrade /
   before_downgrade` card.

Locked before implementation.

---

## Part 1 — The startup row wears the harness's own mark

### Functional Behavior

- New `src/components/harnessMarks.tsx`, built like `connectorLogos.tsx`: inline
  SVG, `currentColor`, `aria-hidden`, one record keyed by harness id, plus a
  resolver. The mark is **never load-bearing** — the harness is always also named
  in the label beside it, so an unrecognised id reads identically.
- Marks are radially symmetric so a slow rotation reads as scintillation rather
  than as a spinning logo: `claude` an eight-arm asterisk (the ✳ Claude Code
  itself draws), `codex` a six-arm rounded star off a hollow centre, `opencode` a
  twelve-tick ring around a solid core. Any other harness id — the id space is
  open, see `harnessLabel` — gets a gapped arc ring, which is a spinner and needs
  no brand knowledge.
- Arm counts are chosen by how each figure resolves at 14px, the only size this
  row uses. Ten arms through the centre muddies into a blob there; eight stays an
  asterisk. OpenCode's ticks are twelve rather than eight so it never reads as
  Claude's mark at a glance.
- New tokens `--harness-claude` / `--harness-codex` / `--harness-opencode`
  (light + dark) exposed through `@theme inline` as `--color-harness-*`. Unknown
  ids tint with `text-muted-foreground`. The tint is transient — it exists only
  while a session starts — and it identifies the harness, so it does not make the
  achromatic ladder chromatic.
- New keyframes `harness-turn` (slow linear rotation) and `harness-breathe`
  (opacity) plus a `.harness-mark-live` class in `src/index.css`. The stylesheet's
  global `prefers-reduced-motion` rule already freezes CSS animation; the
  component *also* withholds the class when `view.reducedMotion`, so the static
  frame is the intended one rather than whatever frame 1 happens to be.
- The label's shimmer treatment is the one the `Thinking…` row already uses. That
  giant arbitrary-value string is folded into a `.text-shimmer` class in
  `src/index.css` and both call sites use it — same pixels, one definition.
- Row reads **mark · elapsed · label**, elapsed first, matching the reference.
  The counter is `tabular-nums` so ticking a second never reflows the label.
- The mark is now the node that must never remount across the collapse handoff
  (it takes the shimmer bar's old role). `thinking-shimmer` stays in the
  stylesheet — four other call sites still use it.

### Unit Tests

- `harnessMarks.test.tsx`
  - a known harness renders its own mark, tinted with its own token class
  - an unknown harness id renders the fallback arc and the muted tint
  - every mark is `aria-hidden` (the label carries the meaning)
- `AgentConversation.test.tsx`
  - the startup row renders the mark for the session's harness
  - the mark carries `harness-mark-live` normally and does **not** under
    reduced motion
  - elapsed precedes the label in the rendered text
  - collapsed (streaming has begun) keeps the mark and drops the label

## Part 2 — A trivial chat is not asked for a handoff checkpoint, and the ask never reaches the transcript

### Functional Behavior

- **The reply is plumbing.** While a compaction is pending, the assistant
  message and reasoning frames of the internal checkpoint turn are neither
  persisted nor published as agent events; only `message.completed` is consumed
  by `CompactionController::handle_output`. This includes streamed deltas, which
  would otherwise briefly put raw JSON or a refusal in the live transcript. The
  maintenance-turn marker remains active after a timeout/cancellation until that
  provider turn ends, so a late reply cannot race the terminal compaction entry
  and become ordinary assistant prose. This follows the
  delegation/peek/steer precedent in the same reader loop: a machine block is not
  something to read. Applies to a valid checkpoint too — raw checkpoint JSON was
  equally a leak. The frame is recognised *before* the store call rather than
  hidden afterwards, because a persisted-then-hidden entry is still in the forest
  and the forest is what a reconnecting client replays.
  The reader-loop tests drive synthetic provider frames directly and verify valid
  and invalid replies, streamed deltas, and a late reply after cancellation. If
  the root adapter exits before the maintenance turn completes, its reader-exit
  cleanup records the compaction failure, clears the pending marker, and stops
  the session; a later resumed user turn must never inherit checkpoint mode.
  Likewise, if the live adapter ends the maintenance turn without producing an
  assistant checkpoint response, Bridge records the missing reply as a failure
  before returning the session to ordinary turns.
- **The ask is attributable and demands no fabrication.** `checkpoint_prompt`
  names Bridge session maintenance as the asker, says why it is being asked, and
  states that empty `decisions` and `filesTouched` are valid. An honest agent with
  nothing to report can now answer honestly instead of refusing. Because this
  maintenance request runs in the existing tool-capable agent session, it also
  explicitly forbids tool calls, commands, file changes, delegation, and every
  other side effect: the turn may only return the checkpoint JSON.
- **Nothing to compact means no round trip.** `plan_switch_summary` skips when the
  active branch's token estimate is below `SWITCH_SUMMARY_MIN_TOKENS`: below that
  floor `start_chat`'s mechanical projection already carries the whole
  conversation, so the round trip buys nothing and can only cost. The existing
  "meaningful work" gate stays; this is a floor under it, not a replacement.
- **The maintenance card reads in English and once.** `compaction.requested` maps
  its `reason` through `compactionReasonLabel` ("Before switching models"), and
  the `<code>` block below the body text goes away — `data.reason` was the only
  source that text ever had, so the block could only ever repeat it, which is how
  a switch came to show `before_downgrade` twice. `compaction.failed` is left
  alone: its `reason` is the failure text, not a reason code — same field name,
  different field. An unrecognised code passes through unchanged, because the
  host owns that set and may add to it.

### Unit Tests

- Rust `compaction_controller`
  - `checkpoint_prompt` names Bridge as the asker and permits empty arrays
  - the prompt explicitly forbids tools, commands, file changes, delegation,
    and other actions during the maintenance turn
  - the prompt still round-trips through `Checkpoint::parse_and_validate`
- Rust `sessions`
  - the floor lands between the two cases it exists to separate: a greeting
    estimates under `SWITCH_SUMMARY_MIN_TOKENS`, a branch with real history over
    it; `plan_switch_summary` itself must return `None` without appending a
    request below the floor and `Some` above it.
- Rust `live_turn`
  - valid and invalid checkpoint replies never persist or publish assistant text
  - streamed checkpoint message/reasoning frames never enter the transcript
  - cancellation keeps the maintenance-turn marker until `turn.completed`, so a
    late checkpoint reply is still suppressed
  - root adapter exit before `turn.completed` records a compaction failure and
    clears both the pending marker and `checkpointing` session status
  - a completion-only maintenance turn clears an attempt-zero pending request,
    so the next real assistant reply is not consumed as checkpoint output
- Frontend `conversation.test.ts`
  - `compaction.requested` carries an English reason, and the raw reason is not
    duplicated into `data.reason` rendering
- Frontend `AgentConversation`
  - a compaction card renders its reason once

### Integration / Smoke

- `bun run check`, `bun run test`, `bun run build` all green.

## Manual Tests (reviewer)

1. Cold-start a Claude chat → asterisk mark, tinted, turning; `3s · Waiting for
   Claude to answer…`; hands off without the mark restarting.
2. Same for Codex and OpenCode; then an unknown harness id → arc spinner.
3. macOS reduce-motion on → mark present, frozen, label static.
4. Say "hi", switch the model → no agent prose in the transcript, no compaction
   card, switch completes.
5. Have a real working session (tools, files), switch the model → checkpoint
   still happens and still carries a summary; no JSON in the transcript.

## E2E

N/A — desktop app; manual + unit coverage per above.
