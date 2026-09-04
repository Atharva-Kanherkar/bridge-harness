# feat/issue-488-normalized-events — Test Contract

One closed, typed event union at the TypeScript boundary; one pure reducer over
it; pure UI above that. Locked before implementation.

## Vocabulary this contract covers

The wire vocabulary is owned by the Rust normalizers
(`src-tauri/bridge-core/src/agent.rs`, `acp_events.rs`) and by the durable
writer (`store::session_event_in_transaction`, which rewrites
`message.completed` to `user.message`/`assistant.message` and refuses to
persist deltas, `*.progress`, `turn.*`, `usage.updated`, `plan.updated`,
`question.settled`). It was enumerated, not guessed:

| Family | Wire kinds |
| --- | --- |
| message | `message.delta`, `message.completed`, `user.message`, `assistant.message` |
| thinking | `reasoning.delta`, `reasoning.started`, `reasoning.completed`, `reasoning` |
| tool | `tool.started`, `tool.progress`, `tool.completed`, `command.started`, `command.output_delta`, `command.completed`, `command.failed` |
| diff | `file_change.started`, `file_change.completed`, `diff.delta`, `diff.updated` |
| plan | `plan.updated` (and any `plan.*`) |
| approval | `approval.requested`, `approval.resolved`, `approval.settled` |
| permission | `permission.requested`, `permission.resolving`, `permission.resolved` |
| question | `question.requested`, `question.asked`, `question.replied`, `question.rejected`, `question.resolved`, `question.settled` |
| delegation | `delegation.spawned`, `delegation.resumed`, `delegation.requested`, `delegation.approved`, `delegation.rejected`, `delegation.blocked`, `delegation.result`, `delegation.steered`, `worker.result` |
| artifact | `artifact.created` (and any `artifact.*`) |
| history | `checkpoint`, `compaction`, `compaction.requested`, `compaction.failed`, `branch.summary`, `handoff.brief` |
| turn | `turn.started`, `turn.completed`, `turn.failed` |
| session | `session.started`, `session.status`, `session.idle`, `session.error`, `session.model_changed`, `session.resume_failed`, `session.diff`, `session.context`, and any other `session.*` |
| notice | `workspace.stale_base`, `workspace.created`, `model.changed`, `model.rerouted`, `effort.changed`, `mode.updated`, `commands.updated`, `config.updated`, `todo.updated`, `extension.handled`, `runtime.failed` |
| error | `error` |
| usage | `usage.updated` |
| raw | `provider.unknown`, any `provider.*`, any `raw.*`, any entry whose `contextVisibility` contains `raw` |

Anything outside those families is **unknown** and is handled by rule 12.

## Functional Behavior

1. **One closed discriminated union.** `src/transcript/events.ts` exports
   `TranscriptEvent`, a union discriminated on `type`, with a shared
   `TranscriptEnvelope` (`sessionId`, `sequence`, `eventId`, `itemId`,
   `entryId`, `createdAt`, `origin`, `key`). Every semantic field a reducer or
   a card reads is a named, typed property on its variant. The untyped
   remainder of a provider payload travels in exactly one clearly-named
   escape field, `providerData`, and nothing under `src/transcript/` branches
   on it. **Documented divergence from the issue text:** the issue asks that
   `Record<string, unknown>` appear nowhere but `UnknownEvent.raw`.
   `ConversationItem.data` is read by roughly forty call sites in
   `AgentConversation.tsx` for card-level detail (`childBlocked`, `willRetry`,
   `divergence`, `actions`, `questions`, `attachments`, …). Typing all of those
   is a separate change; this branch confines the bag to `providerData` and
   forbids semantic branching on it inside `src/transcript/`.

2. **One codec.** `src/transcript/codec.ts` exports
   `normalizeAgentEvent(raw: AgentEvent): TranscriptEvent` and
   `normalizeSessionEntry(entry: SessionEntry): TranscriptEvent | null`.
   Both produce the same union. All per-harness payload archaeology
   (`namedToolFacet`, `readExitCode`, `readPatch`, `readPath`, `readOutput`,
   `reasoningDisplayText`) runs here, once, at ingestion.

3. **One pure reducer.** `src/transcript/reducer.ts` exports
   `reduceTranscript(events: TranscriptEvent[]): ConversationItem[]`: state in,
   state out, no side effects, no harness branches, no wire-kind strings.
   `reduceConversation` and `projectSessionConversation` in `src/conversation.ts`
   become thin wrappers (normalize, then reduce) with unchanged signatures, so
   `App.tsx` and `AgentConversation.tsx` call sites are untouched.

4. **Transient frames keep their causal position.** Sequence-0 frames anchor
   just after the last durable frame seen (`causalAnchor` when the live buffer
   coalesced them), and an unanchored live window sorts after all history.

5. **Live and durable agree on identity.** A tool call, approval, permission or
   question has the same `identity` whether it arrived live or was replayed
   from the forest. Durable items keep `entry:<id>` keys; live items keep the
   provider `itemId` as their key.

6. **Tool calls fold.** `*.started` + `*.progress`/`*.output_delta` +
   `*.completed` sharing an `itemId` produce exactly one `ConversationItem`,
   live and durable alike. Output deltas append; the completion replaces
   status/title and merges payload.

7. **Interactions fold.** `approval.resolved` folds onto the
   `approval.requested` row it names through `requestEventId`; the same holds
   for `permission.*` and `question.*`. The pending row keeps its own
   `eventId` so its action buttons still resolve.

8. **A typed tool facet is computed once.** `ConversationItem.tool?:
   ToolCallDisplay` is stamped by the reducer from the codec's typed facet.
   `toolCallDisplay(item)` returns that stamped facet when present, so
   `AgentConversation.tsx` needs no change and the archaeology no longer runs
   per render.

9. **Harness-shaped payloads all reduce to the same structure.** For one
   logical turn (user message → thinking → command with output and exit code →
   file read → patch → assistant message → turn complete), Codex, Claude,
   Cursor (ACP) and OpenCode reduce to the same sequence of
   `ConversationItem` `type`/`status` values and the same tool verbs.

10. **Compaction envelopes stay internal.** Assistant prose emitted between
    `compaction.requested` and `compaction`/`compaction.failed`, or stamped
    `bridgeInternalOrigin: "compaction"`, never renders — live or durable.

11. **Lifecycle plumbing never renders.** `session.*` (except
    `session.model_changed`), `turn.*` and `usage.updated` produce no item.
    `turn.completed`/`turn.failed`/`session.idle` settle streaming thinking to
    `completed`.

12. **Unknown kinds are visible, never `activity`.** A kind outside every
    family in the table above becomes `UnknownEvent` carrying `raw`, reduces to
    a `ConversationItem` of type `"raw"` (collapsed and inspectable, the same
    surface raw provider frames already use), and in dev
    (`import.meta.env.DEV`) reports itself through `console.error` with the raw
    kind. It never silently becomes `activity`.

13. **Type-level boundary.** `AgentEvent.kind` is an opaque `WireKind`.
    Deliberately **not** a branded string (`string & {…}`): TypeScript's
    comparability rule still lets a branded string equal a literal, so
    `event.kind === "tool.started"` would have kept compiling. Declared as a
    handle instead, so comparison, `switch`, `startsWith` and template
    interpolation are all type errors. `readWireKind(kind)` is the one way to
    open one and `asWireKind(kind)` the one way to mint one (the `api.ts`
    decode boundary, the mock layer, test fixtures). The type error itself is
    locked by a `@ts-expect-error` assertion in the gate test, so relaxing the
    brand fails `tsc`.

14. **Lint gate.** A test fails the build if a transcript rendering component
    branches on harness identity or reads wire kinds. Identity display sites
    (`src/components/harnessMarks.tsx`, `harnessLabel` in `src/utils.ts`) are
    allowed and named in the test.

---

## Unit Tests

### `src/transcript/codec.test.ts`
- `maps every wire kind in the vocabulary table to a typed event` — table-driven
  over the kinds listed above; asserts no `unknown` for any of them.
- `reports an unfamiliar kind instead of calling it activity` — asserts the
  returned variant is `unknown`, that `raw` carries the original kind, and that
  `console.error` was called under `import.meta.env.DEV`.
- `reads an exit code wherever a provider puts it` — camelCase, snake_case,
  nested `state.metadata.exit`.
- `reads a patch wherever a provider hangs it` — `data.patch`, `state.diff`,
  per-file `changes[].diff` joined in order, and a body that is unmistakably a
  diff; never for a read.
- `reads Codex summary-only reasoning as the thought text`.
- `re-encodes a durable entry into the same union as its live twin` — the
  live/durable pair for a tool call normalizes to the same variant and itemId.
- `refuses an unsupported semantic schema version`.

### `src/transcript/reducer.test.ts`
- `folds a tool lifecycle into one item` (started → output deltas → completed).
- `folds an approval resolution onto its request`.
- `settles streaming thinking on turn completion`.
- `keeps separate thinking cards across turns when itemId is null`.
- `orders sequence-0 frames where they streamed`.
- `surfaces an unknown event as a raw item, never activity`.

### `src/transcript/golden.test.ts` (fixtures loaded by `golden.ts`)
- `normalizes every <harness> frame into the union, never into unknown` — reads
  `src/transcript/fixtures/{claude,codex,cursor,opencode}.json` and asserts the
  exact normalized `type` sequence per harness. The four sequences are **not**
  identical, and the table in the test says why: Claude streams no tool output,
  OpenCode re-sends a whole running part instead of a delta, and ACP has no
  "thought completed" frame at all.
- `puts <harness>'s three calls on the same three surfaces` — activity,
  activity, diff, by item id rather than by frame.
- `reduces the <harness> turn to the same rows` — the seven-row structure
  (`type`, `role`, `status`, tool verb) is identical for all four.
- `says the same thing about <harness>'s tool calls` — command, output, target,
  path and patch agree across all four.
- `reports <harness>'s exit code only where the protocol has one` — Codex and
  OpenCode carry one; Claude's `tool_result` and ACP have no field for it.
- `reads the same prose out of all four`.

### `src/transcript/golden.render.test.tsx`
- `draws the same transcript for every harness` — jsdom, `react-dom/client` +
  `act`, mounts `AgentConversation` on each harness's raw stream and compares
  the ordered row shapes and count.

### `src/transcript/harnessBranchGate.test.ts`
- `no transcript rendering component branches on harness identity` — reads
  `src/components/AgentConversation.tsx`, `DiffView.tsx`, `workerPanel.ts`,
  `TranscriptPane.tsx` and fails on `/harness\s*[!=]==\s*["']/`.
- `no transcript rendering component reads a wire event kind`.
- `AgentConversation imports only normalized types from the transcript layer`.

### Regression net (must stay green, unchanged in behavior)
- `src/conversation.test.ts` — all cases.
- `src/agentEvents.test.ts` — all cases.
- `src/components/AgentConversation.test.tsx` — all cases.

---

## Integration / Functional Tests
- Golden streams per harness for one logical turn, reduced and rendered
  (see `golden.test.ts` above).
- Durable/live merge: a durable spawn folded with a live result still produces
  one worker panel.

---

## Smoke Tests
- `bun run build` — `tsc -b && vite build`.
- `bunx vitest run` — the whole frontend suite.

---

## Known parity divergences

Recorded here rather than skipped, per the issue:

1. **`worker.result`.** Live it reduced to `activity`; durable it reduced to
   `delegation`. Unified to `delegation` on both paths, matching the durable
   reading and the worker-panel fold that depends on it.
2. **Message text cleaning.** Live applied
   `stripBridgeFences(stripWorkerResultBlocks(text))` to every message
   including the user's; durable applied `stripBridgeFences` to assistant
   messages only. Unified to the live rule on both paths.
3. **Empty messages.** Live dropped messages whose text is blank; durable kept
   them. Unified to dropping them.
4. **`provider.unknown`.** Live dropped it; durable rendered a collapsed raw
   card. Preserved as-is: the codec emits a `raw` event either way and the
   reducer renders it only when it came from the forest, because a live raw
   frame has no stable identity to reconcile against its durable twin.
5. **Unknown kinds** now render as a collapsed raw card instead of a generic
   activity row (rule 12).
6. **`checkpoint`, `compaction*` and `branch.summary` arriving live** reduced to
   generic activity rows; only the forest projection gave them their own cards.
   Unified to the card on both paths.
7. **An orphan resolution** — an `approval.resolved` or `permission.resolved`
   whose request is not on the branch, or has scrolled out of the live window —
   is dropped on both paths. Live already dropped it; the forest projection
   grew a bare "Approval resolved" row saying nothing.
8. **ACP tool categories.** Cursor sends every call as `tool.*` with the
   category on `data.kind`, so an ACP read used to render as an anonymous
   "Using a tool" and an ACP edit as a plain tool row. The codec now reads
   `data.kind` for the verb and puts an `edit` on the diff surface.
9. **ACP output, paths and diffs.** ACP hangs a call's output on the update's
   content blocks, its files in `locations`, and its edit as a `diff` block
   rather than a patch. All three are now read, and the diff block is
   synthesized into a unified patch the same way the Rust side already does for
   Claude.
10. **OpenCode tool names and arguments.** OpenCode names the tool `tool`, not
    `name`, and nests its arguments under `state.input`. Both are now read, so
    an OpenCode read is a read rather than an anonymous tool call.

Divergences 8 to 10 are behavior changes on the Cursor and OpenCode transcripts
specifically. They are what the golden test needs in order to assert real
parity rather than assert documented brokenness.

11. **ACP's thought stream, replayed.** Cursor's reasoning frames are
    `reasoning.delta` only (`acp_events.rs` never emits a `reasoning.completed`
    for a thought chunk), so the live window's two thought cards are built
    from deltas alone. The durable writer never persists a kind ending in
    `.delta` (`store::session_event_in_transaction`), so a Cursor turn
    replayed from the forest has no thought cards at all. The golden parity
    test (`golden.test.ts`) asserts this explicitly rather than asserting
    reasoning parity for cursor.
