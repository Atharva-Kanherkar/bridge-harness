# feat-compaction-ownership-and-checkpoint-contract — Test Contract

Locked before implementation. Covers GitHub issue #530 ([5/6] of #525): gap **G6**
(who owns live-context compaction) and gap **G5** (checkpoint contract hardening).
Siblings #526–#529 are merged; this contract inherits their scope notes and does
not reopen them.

The decision this contract implements is **option (a): the harness owns the hot
window, Bridge owns the record.** Evidence gathered before locking:

| Fact | Source |
| --- | --- |
| Bridge pressure compaction has never fired in 30 successful compactions | local `bridge.db`, `session_entries.kind='compaction'` grouped by `$.reason`: phase boundary, switch, shutdown only |
| A Bridge checkpoint frees zero provider tokens on a hot session | `compaction_controller::record_checkpoint` writes forest entries and moves `session_heads`, never touches the adapter |
| Claude reports its own compaction with token figures | Claude Agent SDK 0.3.261 `SDKCompactBoundaryMessage`: `trigger`, `pre_tokens`, `post_tokens`, `duration_ms` |
| Codex reports its own compaction | app-server 0.153.4 `ContextCompactionThreadItem`; `thread/compacted` is present but marked deprecated |
| OpenCode reports its own compaction | server 1.18.3 `session.compacted` event, payload `{sessionID}` |
| All three accept a compaction command | Claude slash command on the input stream; Codex `thread/compact/start`; OpenCode `POST /session/{id}/summarize` |
| Only Claude's command accepts a focus | `ThreadCompactStartParams` is `{threadId}`; OpenCode's body is `{providerID, modelID, auto}` |

## Scope

In scope: the ownership decision and its doc, surfacing native compaction as
durable history, honest compaction cards, `/compact [focus]` forwarding, and the
G5 checkpoint contract rewrite.

Out of scope, deliberately, with reasons recorded here so a reviewer does not read
them as omissions:

- **Computing `context_percent` for Codex and Claude.** That is option (b)'s
  instrument. Under (a) nothing consumes a Bridge-computed pressure gauge, and
  making the column non-NULL for these harnesses would newly expose G11
  (`begin_pressure_compaction` reads the latest row with no thread filter), which
  is #531's scope.
- **Seeding a Bridge checkpoint from Claude's `PostCompact` hook summary.** Real
  value, but it needs a hook channel through the sidecar and it changes what a
  checkpoint's provenance means. Follow-up.
- **Restarting a provider process to realise a Bridge compaction.** Rejected by
  the decision itself: it would discard the native session and the prompt cache
  that #527 and #528 exist to preserve.

## Functional Behavior

### 1. The decision is recorded

- `docs/compaction-and-resume.md` states that within a live process the harness
  owns its context window and Bridge does not compact it, names the three native
  mechanisms, and gives the reasons from the table above.
- The doc states why a direct chat never auto-compacts: its harness owns the live
  window, so Bridge's boundaries are phase, switch, shutdown, and manual only.
  This is the documented half of G6's "remove the exclusion or document why".
- The doc states that a Bridge checkpoint is a cold-start payload and that
  `ContextPressure` remains reachable only for a harness that reports a context
  gauge and has no compaction of its own.

### 2. Native compaction becomes durable history

- A new normalized event kind `context.compacted` carries `harness` plus, when
  the provider reports them, `trigger`, `preTokens`, `postTokens`, `durationMs`.
  It is durable: `store::session_event` stores it like any other conversation
  entry, in stream order.
- Claude: a `system` message with `subtype: "compact_boundary"` normalizes to
  `context.compacted` with `trigger` from `compact_metadata.trigger` and the three
  token/duration figures when present. It is no longer dropped by the
  `normalize_claude_system` catch-all.
- Codex: a `contextCompaction` thread item on `item/completed` normalizes to
  `context.compacted`. The `item/started` half of the same item produces no event:
  the boundary is a completed fact and the opening half carries nothing a card can
  show. `thread/compacted` leaves the swallow list and normalizes to the same kind,
  so an older binary is covered too.
- OpenCode: `session.compacted` normalizes to `context.compacted` instead of
  falling through to `provider.unknown`.
- A native compaction never begins, cancels, or satisfies a Bridge compaction. No
  `compaction.requested`, `compaction`, or `compaction.failed` entry is written in
  response to one, and `session_heads.active_entry_id` is not moved by one.

### 3. The compaction card tells the truth

- "Context compacted" is drawn only from a `context.compacted` entry, live and on
  replay. When the provider reported token figures the card names them; when it
  did not, the card says which harness compacted and nothing more.
- A successful Bridge checkpoint on a hot session reads "Checkpoint saved", not
  "Context compacted". Its meaning is that history was summarised for a later cold
  start, which is what it does.
- The transcript's in-flight compaction state follows the provider: Claude's
  `status: "compacting"` frame, and Bridge's own `compaction.requested` for a
  Bridge checkpoint. Neither claims the other's state.
- `compaction.failed` copy is unchanged. A Bridge checkpoint that fails still never
  claims history was lost.

### 4. `/compact [focus]` reaches the harness

- `AdapterRuntime` gains `native_compaction()` returning `Unsupported`,
  `WholeConversation`, or `WithFocus`, and `compact_native(focus)`. The default is
  `Unsupported` and the default `compact_native` errs, so a harness that has no
  compaction command fails at the seam rather than silently doing nothing.
- Claude answers `WithFocus` and forwards `/compact` or `/compact <focus>` as a
  user turn, which is how the Agent SDK receives a slash command.
- Codex answers `WholeConversation` when its protocol schema exposes
  `thread/compact/start`, discovered the same way `thread/fork` and
  `thread/resume` already are, and `Unsupported` otherwise. It sends
  `thread/compact/start` with the thread id.
- OpenCode answers `WholeConversation` when it knows its provider and model, and
  posts `/session/{id}/summarize` with `providerID` and `modelID`.
- Cursor and Grok answer `Unsupported`.
- `/compact focus …` against a `WholeConversation` harness runs the compaction and
  says the focus was not applied, naming the harness. Nothing is silently dropped.
- `/compact` against an `Unsupported` harness falls back to the Bridge checkpoint
  that exists today.
- A forwarded `/compact` writes no `compaction.requested` entry.

### 4b. A forwarded compaction is a turn, and asking costs no lock

Added after review, from two findings on the first push.

- The capability has two halves. The static half, "does this harness have a
  compaction command at all", is answered through `AdapterRegistry` before the
  adapters mutex is taken, because Codex answers it by launching
  `codex app-server generate-json-schema`. Holding the process-wide mutex across
  that would stall every other session's adapter I/O behind one chat's first
  `/compact`. The runtime's `native_compaction()` then refines it with
  per-session state, under a single acquisition that also dispatches.
- A harness that answers no to the static half never reaches the runtime.
- A forwarded compaction marks the session `status='working'` before returning.
  The provider's `turn.started` is asynchronous, and that window is exactly
  where a message typed immediately after would be routed as a new turn and
  collide with the compaction in flight. This is the same pessimism
  `deliver_prepared_input` already applies, and it asserts something true:
  Claude reads the slash line off its input stream, Codex and OpenCode each run
  theirs as a turn.
- It is never `checkpointing`. That status suppresses content frames as
  protocol traffic, which would hide what the harness says while compacting.
- The mark is self-healing. A `context.compacted` entry releases it when no
  turn id is tracked, so a harness that reports a boundary without turn
  lifecycle cannot leave a session claiming a turn forever, which is the #261
  shape: queued input waiting on a boundary that never arrives. A boundary
  during a real turn carries an active turn id and is left alone.

### 4c. A released compaction owes what a turn's end owes

Added after the second review round, from a finding on 4b's own fix.

- Marking the session `working` made input queue, which is correct, but the
  release only wrote the status. It notified no listener and drained nothing, so
  a harness reporting a boundary without turn lifecycle left the composer busy
  and left the queued message undelivered against a database that already read
  idle.
- The release is now a named rule, `release_native_compaction`, that reports
  whether it fired. When it fires, the session drains its queue and publishes
  `StateChanged`, which is the same finish work `turn_completed` runs. That
  input was queued *because* the forward marked the session working, so the
  release is the only thing that will let it through.
- A batch carrying both a real `turn.completed` and a release drains once, not
  twice.
- `drain_queued_input` re-checks idleness itself, so the release's drain is a
  no-op if anything else has claimed the session in between.

### 5. The checkpoint contract stops failing on shape

- Bridge fills `sourceAgent`, `firstRetainedEntryId`, `tokensBefore`, `reason`,
  and `schemaVersion` itself. The prompt asks the model only for `summary`,
  `decisions`, `filesTouched`, and open work, and never asks it to echo a UUID.
- The reply is parsed by extracting its first balanced JSON object, so a fenced
  block, a preamble, or trailing prose parses. A reply with no JSON object still
  fails.
- The attempt-0 prompt carries the durable evidence list. The repair prompt is no
  longer the first time the model sees it.
- An evidence gap is repaired, not fatal: Bridge appends the missing decisions and
  files and stamps `provenance: "agent+controller"`. A checkpoint that matches the
  evidence keeps its `agent` provenance.
- `decisions` and `filesTouched` are deduplicated, preserving first-seen order,
  instead of being rejected for duplicates.
- The prompt no longer opens with a bracketed maintenance tag that models answer
  with reasoning or refuse. It states plainly that Bridge is asking, that the
  reply is bookkeeping, and that no tools may be called.
- The failure kinds that remain reachable are timeout, no assistant response, and
  a reply with no JSON object. Metadata echo and fenced JSON are no longer
  failure modes at all.

## Unit Tests

Rust, `bridge-core`:

- `agent.rs`: `claude_compact_boundary_becomes_a_durable_context_compaction`
  asserts kind, harness, trigger, and the token figures; a boundary without
  `post_tokens` omits the key rather than inventing a zero.
- `agent.rs`: `codex_context_compaction_item_surfaces_once` asserts
  `item/completed` yields one `context.compacted` and `item/started` for the same
  type yields none.
- `agent.rs`: `codex_thread_compacted_is_no_longer_swallowed` asserts the
  deprecated notification maps to `context.compacted`.
- `agent.rs`: `opencode_session_compacted_is_not_a_provider_unknown`.
- `agent.rs`: the existing swallow-list assertion is updated so it cannot pass
  with `thread/compacted` still listed.
- `store.rs`: `context_compaction_is_durable` asserts a `context.compacted` event
  writes a `session_entries` row and is not treated as transient.
- `session_forest.rs`: a stored `context.compacted` entry reads back through
  `validate_stored_entry` unchanged.
- `context.rs`: `project_render_entry` keeps `context.compacted` visible to a
  projection, because it is conversation history, and the newest Bridge
  `compaction` boundary is still the only projection boundary.
- `adapters.rs`: `native_compaction_defaults_to_unsupported` and
  `default_compact_native_errs`.
- `codex_adapter.rs`: `native_compaction_capability_is_discovered_from_protocol_schema`,
  including the negative case where the schema lacks `thread/compact/start`.
- `codex_adapter.rs`: `compact_start_request_carries_the_thread_id`.
- `opencode_adapter.rs`: `summarize_body_carries_provider_and_model`, and a
  runtime with no model answers `Unsupported`.
- `claude_adapter.rs`: `compact_forwards_the_focus_as_a_user_turn`, asserting the
  exact text for both the bare and the focused form.
- `slash.rs`: `/compact` still dispatches `Compact` with the focus preserved; the
  existing focus test keeps its assertion.
- `compaction_controller.rs`: `checkpoint_prompt_asks_only_for_semantics` asserts
  the prompt contains no UUID, no `tokensBefore` integer, and no instruction to
  copy metadata, and does carry the evidence list on attempt 0.
- `compaction_controller.rs`: `fenced_checkpoint_json_is_accepted`,
  `checkpoint_with_a_preamble_is_accepted`, `reply_without_json_still_fails`.
- `compaction_controller.rs`: `evidence_gap_is_augmented_not_rejected` replaces the
  repair-on-evidence-gap test and asserts the committed entry contains the missing
  items and `provenance: "agent+controller"`.
- `compaction_controller.rs`: `duplicate_decisions_are_deduplicated_in_order`.
- `compaction_controller.rs`: existing tests that hand-build a reply echoing
  controller metadata are rewritten to the semantics-only reply shape.
- `context.rs`: `Checkpoint::parse_and_validate` keeps rejecting a wrong
  `sourceAgent` when one is supplied by the caller, so the ownership check survives
  the metadata move.

Frontend, Vitest:

- `src/transcript/codec.test.ts`: a live `context.compacted` frame and a durable
  `context.compacted` entry both decode to a compaction card whose title is
  "Context compacted"; the text names the harness, and names token figures only
  when the payload carried them.
- `src/transcript/codec.test.ts`: a durable `compaction` entry decodes to
  "Checkpoint saved".
- `src/transcript/reducer.test.ts`: a native compaction does not set or clear the
  `compacting` fold that belongs to a Bridge checkpoint request.
- `context.compacted` is a known kind: no `reportUnknown` call for it.

## Integration / Functional Tests

- A Codex `contextCompaction` item driven through the live-turn handler appends
  exactly one durable entry and leaves `compaction.requested` count at zero.
- `/compact` on a session whose runtime answers `WholeConversation` calls
  `compact_native` once, appends no `compaction.requested`, and emits the
  focus-not-applied line only when a focus was given.
- `/compact` on a session whose runtime answers `Unsupported` still produces the
  Bridge checkpoint request exactly as it does today.
- A checkpoint reply wrapped in a ```json fence commits a `checkpoint` and a
  `compaction` entry, and the `compaction` payload carries the controller's
  `firstRetainedEntryId` and `tokensBefore` even though the model never saw them.
- A checkpoint reply omitting a durable worker decision commits with the decision
  appended and mixed provenance, and writes no `compaction.failed`.

## Smoke Tests

- `bun run check` clean.
- `bun run test` green: sidecar `node:test`, Vitest, `cargo test --workspace`.
- `bun run build` green.
- `cargo test -p bridge-protocol` green: no protocol artifact drift, since this
  change adds no wire method.

## E2E Tests

N/A. Bridge is a local Tauri app with no HTTP surface, and a real provider
compaction needs a long live session. Covered by the manual notes instead.

## Manual Notes

- `bridge exec --json` on a Codex chat, then `/compact`, and confirm one
  "Context compacted" card and no "Compaction failed" entry.
- On a long Claude chat, let autocompact fire and confirm the card names
  pre and post tokens.
- Ledger check after a week of use, the query from #525:
  `SELECT kind, count(*) FROM session_entries WHERE kind IN ('compaction','compaction.failed','context.compacted') GROUP BY kind;`
  Success must exceed failure. Acceptance criterion 5 of #530 is observational and
  cannot be closed by this PR.

## Divergences from this contract

Recorded after implementation, so the contract keeps its pre-implementation
shape and the differences are visible rather than quietly reconciled.

**Test names.** Several tests were named for what they pin rather than for the
mechanism, e.g. `codex_context_compaction_item_surfaces_once` shipped as
`codex_context_compaction_item_is_recorded_once_per_turn`, and
`context_compaction_is_durable` as `a_native_compaction_is_durable_history`.
Coverage matches; the names do not.

**Merged tests.** The `session_forest.rs` read-back check is folded into
`store::a_native_compaction_is_durable_history`, which writes the entry and
reads it back through `validate_stored_entry` in one case rather than asserting
the same thing twice. The two `adapters.rs` default checks are one test plus a
second on the three-state capability.

**`Checkpoint::parse_and_validate` is deleted, not kept.** The contract said it
would keep rejecting a wrong `sourceAgent`. Once Bridge fills `sourceAgent`
itself, ownership is guaranteed by construction and that function had no
production caller left, so keeping a stricter second parser would have been an
invitation to reintroduce the whole-message parse. `Checkpoint::validate` still
enforces the mismatch for any caller that supplies an expected agent, and
`a_stored_boundary_keeps_its_own_invariants` covers it.

**Two seams extracted to make contract tests possible.**
`survives_checkpoint_turn` in `live_turn.rs`, `compact_start_params` in
`codex_adapter.rs`, and `compaction_support` in `opencode_adapter.rs` were
inline expressions with no way to assert on them. Each is now a named function
with a test, following the pattern the ACP body-only fold already set.

**One contract claim that was wrong about the mechanism.** Section 2's last
bullet said `session_heads.active_entry_id` "is not moved by" a native
compaction. It is: every durable entry advances the head, an ordinary assistant
message included, because that is how the active branch is tracked. What the
bullet was protecting is real and does hold: a native boundary is not a
*projection* boundary, since only a `compaction` entry moves where
`ContextProjector` starts. The store test asserts the head advances like any
other entry, and `a_native_compaction_stays_in_the_projection` holds the
projection end.

**One contract claim that did not ship.** Section 3 said the transcript's
in-flight compaction state "follows the provider: Claude's `status:
"compacting"` frame". Only the second half of that bullet shipped. A native
compaction neither opens nor closes the maintenance fold a Bridge checkpoint
request opens, which is the correctness half and is tested. Nothing in Bridge
consumes Claude's `compacting` status yet, so the transcript shows no in-flight
state while a harness compacts. Displaying one means adding to the session
status vocabulary and is left as a follow-up rather than claimed here.

**Two additions beyond the contract.**

- `openWork` is stored on the boundary and emitted in the restoration header.
  The contract asked the prompt for open work without saying where it went;
  asking for it and dropping it would have made the field decoration.
- The three Codex schema probes are one `schema_declares(predicate)`. Adding a
  third copy of a process launch and file read was the alternative.

**One pre-existing behaviour recorded rather than changed.** `/compact` on a
session with no live runtime writes `compaction.requested` and then fails with
"checkpoint agent process is not running". That predates this work and is
unchanged; `a_session_with_no_runtime_takes_the_bridge_path_it_always_did`
pins it so a future reader knows it was seen and left alone.

## Verification

- `bun run check`
- `bun run test`
- `bun run build`
- `git diff --check origin/main...HEAD`
