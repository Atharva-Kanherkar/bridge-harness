# fix/cross-harness-switch-and-shell-polish — Test Contract

Field report from dogfooding the merged #363, six defects. The first is the one
Bridge cannot ship with: **every cross-harness model switch bricks the session.**

1. Switching a chat's harness (Claude→Codex, Codex→OpenCode, …) fails every
   later launch with `codex was served by claude.agent-sdk, which this build
   does not have` (and each permutation of that pair).
2. The switch-summary token floor never bites: a greeting-only chat measured
   13,112 tokens because `active_token_estimate` sums the whole branch,
   injected instructions included — so the maintenance turn, the timeout, and
   the failure card all still happen on a chat that has said "hi".
3. Collapsed "Used tools" rows render before and around user messages. They are
   durable machine entries (`session.started`, `session.status`,
   `session.model_changed`) that the live reducer filters but the durable
   projection does not.
4. While a model switch is in flight the row claims the **old** model is
   "reading your message". There is no switch state at all.
5. The first-launch note ("First time opening this chat...") is unwanted. Remove
   it and its plumbing.
6. The Codex mark reads as a generic star, not as the OpenAI knot. The pill can
   also read "OpenCode - OpenCode Go - MiMo V2.5" (harness label duplicated by
   the catalog label).

Locked before implementation.

---

## Part 1 — A harness switch must not brick the session

### Functional Behavior

- `persist_chat_model_selection` clears `backend_id`, `backend_version`, and
  `backend_installation_id` **when the harness changes**. A same-harness model
  change keeps the binding: the same agent will resume under the same backend,
  and the recorded version/installation stay meaningful. A harness change is a
  *different agent* — there is nothing to continue, and the switch already
  clears `provider_session_id` for the same reason.
- Defense in depth for rows the old bug already wrote: `plan_continuation`
  treats a stored binding whose backend is **not a candidate for the session's
  agent but is a registered backend of some other agent** as a cross-switch
  leftover and returns `BindFresh` instead of `BackendUnavailable`. This is not
  silent substitution — a backend that never served this agent cannot be
  "continued", and the fresh bind is exactly what a correctly-cleared row would
  have produced. A backend unknown to *every* agent still errors: that is a
  build downgrade, the case the refusal exists for.

### Unit Tests

- Rust `sessions`: switching harness clears the backend columns; switching only
  the model on the same harness leaves them alone.
- Rust `backend_binding`: a stored `claude.agent-sdk` binding on a session whose
  harness is now `codex` plans `BindFresh` (both through `plan_continuation`
  and end-to-end through `plan_launch`); a stored backend registered for **no**
  agent still refuses with `BackendUnavailable`.

## Part 2 — The floor measures the conversation, not the context

### Functional Behavior

- New `conversation_token_estimate` in `compaction_controller`: the branch
  estimate restricted to conversation kinds (`user.message`,
  `assistant.message`, `worker.result`, `tool.completed`) — the same set the
  `meaningful` gate reads. `plan_switch_summary` floors on **that**, so injected
  instructions can no longer carry a greeting over `SWITCH_SUMMARY_MIN_TOKENS`.
- `tokens_before` recorded in the checkpoint request stays the full-branch
  estimate — it describes context pressure, not conversation size.

### Unit Tests

- Rust `compaction_controller`: a branch whose bulk is non-conversation entries
  yields a small conversation estimate and a large full estimate.
- Rust `sessions`: greeting-plus-injected-instructions stays under the floor;
  a conversation with real tool output clears it.

## Part 3 — Machine entries stay out of the transcript

### Functional Behavior

- `projectSessionConversation` filters the same machine kinds the live reducer
  already filters: `session.*`, `turn.*`, `usage.updated`, `provider.unknown`.
  Today these render as collapsed "used tools" activity groups before and
  around user messages. The live/durable asymmetry means they only ever
  appeared after a reload — which is exactly why they look like a glitch.
- Amended during implementation: `session.model_changed` and
  `provider.unknown` deliberately survive the filter. Two pre-existing tests
  pin them as wanted ("renders a model switch with the fidelity it reports",
  "keeps raw provider entries collapsed and inspectable") — the projection
  filter is a *subset* of the reducer's, not a copy. The model change stops
  being a "used tools" group of one: it leaves activity grouping and renders
  as a hairline divider (`Codex · GPT Luna → Claude · Opus`).

### Unit Tests

- Frontend: durable `session.started` / `session.status` / `turn.completed`
  entries render nothing in a replayed transcript; a message between them
  still renders; a replayed model change renders as the divider and never as
  a tools group.

## Part 4 — The switch gets its own UI state

### Functional Behavior

- `NarrationInput` gains `switchingToLabel: string | null` (and loses
  `firstLaunch`, Part 5). While set, the row is mounted regardless of pending
  work, does not collapse, and reads `Switching to <label>…`; the phase labels
  and the reading-your-message default only apply when it is null.
- `App.changeChatModel` sets a `modelSwitch` state `{harness, label}` before
  awaiting `updateChatModel` and clears it in `finally`; passes it into
  `AgentConversation`, which feeds the label to the narration and wears the
  **target** harness's mark while switching.
- No timers added: the existing 250ms tick already runs the clock; the
  switching row reuses it (elapsed counter appears from 2s as before).

### Unit Tests

- `startupNarration.test.ts`: switching mounts without pending work, wins over
  phase labels, shows elapsed from 2s, unmounts when cleared.
- `AgentConversation.test.tsx`: a switching conversation renders
  `Switching to Opus…` and the target harness's mark.

## Part 5 — The first-launch note goes away

### Functional Behavior

- Delete the note, `showFirstLaunchNote`, `firstLaunch`, and the
  `hasProviderSessionId` prop that fed it. No replacement copy.
- Sweep the strings this feature family added for em dashes: the
  `session.model_changed` detail sentence loses its em dash; narration labels
  and compaction labels already carry none.

### Unit Tests

- Existing first-launch narration tests deleted with the feature; a test pins
  that the note text renders nowhere.

## Part 6 — The Codex mark is the knot, and the pill does not stutter

### Functional Behavior

- The Codex mark becomes a six-lobe interlocking knot in the OpenAI style: six
  stadium outlines at 60° steps around the centre, hexagonal negative space in
  the middle. Still radially symmetric (the turn animation reads as
  scintillation), still `currentColor`, still never load-bearing. Verified by
  eye at 14px before locking the geometry.
- `ChatModelControl`'s compact pill drops the harness prefix when the model
  label already begins with the harness label (case-insensitive), so
  `OpenCode · OpenCode Go · MiMo V2.5` reads `OpenCode Go · MiMo V2.5`.

### Unit Tests

- `harnessMarks.test.tsx`: Codex's figure changed (distinct from the other
  marks, no centre dot, more than one subpath).
- `ChatModelControl.test.tsx`: harness prefix deduplicated when the model label
  starts with it; kept when it does not.

## Integration / Smoke

- `bun run check`, `bun run test`, `bun run build` all green.

## E2E

N/A — desktop app; manual + unit coverage per below.

## Manual Tests (reviewer)

1. Chat on Claude, say hi, switch to Codex → no error, next message launches
   Codex fresh. Switch again to OpenCode → same. (Previously: "was served by"
   error on every permutation.)
2. A session bricked by the old bug (backend column still holding the previous
   harness's backend) launches fresh instead of erroring.
3. Say "hi", switch models → no compaction card, no maintenance turn, switch is
   near-instant; during it the row reads `Switching to <model>…` with the
   target mark.
4. Reload a chat → no "Used tools" rows before or between messages.
5. No first-launch note anywhere.
6. Codex sessions wear the knot; an OpenCode Zen model whose label starts with
   "OpenCode" shows no harness stutter in the pill.
