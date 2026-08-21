# feat/live-worker-steering — Test Contract

Closes the "silent gap" between `Delegating to a worker…` and the result envelope,
and makes a running worker reachable by both the user and its orchestrator.

Scope is the three parts of the issue: (1) a live worker panel inside the
orchestrator conversation, (2) the user steering any live worker, (3) the
orchestrator steering a worker with a `bridge-steer` verb.

No new wire methods. `submit_input` already exists on the protocol; `bridge-steer`
is a fenced model verb parsed host-side. So the protocol mirror / handler-1:1 /
tsgen artifacts are untouched by design, and that is itself an assertion below.

## Functional Behavior

### 1. Live worker panel in the orchestrator conversation

- A `delegation.spawned` (or `delegation.resumed`) conversation item that carries a
  `childSessionId` renders a **live worker panel**, not a static one-line card.
- The panel shows, sourced from `WorkerRuntimeRecord` + the global live event stream:
  - status pill and tone from the existing `workerStatus()` resolver (no second
    status vocabulary),
  - elapsed since the child session started,
  - retry count when non-zero,
  - `progressSummary` when present,
  - `waitingReason` when present,
  - a mini-feed of at most 3 of the worker's most recent legible events, filtered
    from the global stream by `childSessionId`.
- The mini-feed is bounded: at most `WORKER_PANEL_FEED_LINES` lines are ever
  projected, regardless of how many events the worker produced. No new unbounded
  accumulation is introduced (the `ae01432f` memory-bound contract).
- "Expand" hands the child session id up to the app, which renders the existing
  `WorkerDetail` overlay (full feed, fullscreen toggle). "Open session" focuses the
  worker session.
- When the worker's result lands, the **same** item becomes the result card: the
  `delegation.result` event for a `childSessionId` folds into the earlier
  `spawn-<childSessionId>` item instead of appearing as a separate row. Files
  changed / tests / status come from the runtime's `lastResult`.
- Folding preserves every existing result affordance: a classified failure still
  renders `WorkerFailureRow` with its cause and "Retry this task" button.
- A `delegation.result` with no matching spawn item (durable history truncated,
  branch switch) still renders on its own — folding must never swallow a row.

### 2. User steers a running worker

- `submit_input` no longer rejects every session with a `worker_runtime` row.
  A worker-targeted submit is decided by one pure gate:
  - result already `reported` → refused, "already reported its result",
  - lifecycle `checkpointing` → refused (Bridge's own checkpoint turn owns the
    provider; user text must not land inside it),
  - no live adapter runtime → refused, "not running",
  - otherwise → allowed, and routed by the *existing* `session_input::route`
    (`Steer` when the adapter advertises active-turn steering, else `Queue`,
    or `NewTurn` when nothing is in flight).
- A steer delivered to a worker carries a host-authored wrapper on the **provider**
  text restating that the typed `bridge-worker-result` envelope is still required
  and still goes to `report_to_parent`. The **display** text stays the user's own
  words, so the transcript shows what the human actually typed.
- Every accepted user steer emits a `bridge-worker-steered-by-user` routing notice
  to the parent orchestrator through the same adapter seam as
  `notify_parent_child_left_waiting`, carrying the child session id, its label, the
  steer text, and the `fleet` digest.
- Ledger rows: `session.input.steered` on the worker session (existing kind), plus
  `delegation.steer.user_notified` / `delegation.steer.user_undeliverable` on the
  parent depending on whether the orchestrator's runtime took the notice.
- A `delegation.steered` normalized event is written on the parent so the user sees
  a chip in the orchestrator chat saying a steer went in and who sent it.
- Guardrails, asserted: steering does **not** write a `worker_runtime.last_result`,
  does not mark `result_status='reported'`, does not touch `ResultRepairTracker`,
  and does not create or resolve a completion gate.
- UI: the worker focus view's read-only banner is replaced by a composer labeled as
  steering ("Steer this worker…"), and the same composer appears in the
  `WorkerDetail` overlay. The queued-follow-up pill is no longer suppressed for
  worker views.

### 3. Orchestrator steers a worker (`bridge-steer`)

- One fenced ```bridge-steer``` block per assistant message:
  `{"sessionId":"<child>","message":"<guidance>"}`.
- Parsing (`delegation::parse_steer_request`) is strict:
  - both fields required; `deny_unknown_fields`,
  - `sessionId` 1–128 identifier characters (same rule as `bridge-peek`),
  - `message` non-empty after trimming and at most
    `MAX_STEER_MESSAGE_BYTES` bytes,
  - more than one block → `Invalid`,
  - a `bridge-delegate` or `bridge-peek` block is not a steer block.
- The block is stripped from the text the user reads and replaced by
  `_Steering a worker…_` when nothing else remains.
- Delivery is validated host-side: the target must be a child of *this* parent with
  a still-pending result and a live runtime. A stranger's session id, a reported
  worker, or a dead worker is refused and the refusal is fed back to the
  orchestrator (never silently dropped).
- Like `bridge-peek`, delivery happens at turn completion, not mid-frame.
- Ledger rows: `delegation.steer.delivered`, `delegation.steer.undeliverable`, or
  `delegation.steer.invalid`. A `delegation.steered` event on the parent surfaces
  the chip.
- Prompts teach the verb in both `delegation::protocol(0)` and
  `orchestrator::briefing()`: steer to **redirect**, never to ask for status —
  status is `bridge-peek`.

## Unit Tests

### Rust — `bridge-core`

`session_input.rs`
- `worker_steer_gate_refuses_a_reported_worker_and_a_dead_one` — the gate returns
  `AlreadyReported` for `result_status="reported"`, `NotRunning` with no live
  runtime, `Checkpointing` for that lifecycle, and `Ok` for a live pending worker.
- `worker_steer_uses_the_same_route_table_as_any_other_session` — gate-allowed
  worker input still resolves through `route()`; no worker-specific routing table.

`delegation.rs`
- `steer_requests_parse_and_reject_garbage` — valid block parses; missing
  `message`, empty `message`, oversized `message`, bad `sessionId`, unknown field,
  two blocks all `Invalid`; `bridge-delegate` / `bridge-peek` blocks are `Absent`.
- `strip_steer_removes_only_the_steer_block` — surrounding prose survives; a
  `bridge-peek` block in the same message is untouched.
- `protocol_teaches_steering_as_redirection_not_status` — `protocol(0)` names
  `bridge-steer`, and the depth-limited worker protocol does not.

`orchestrator.rs`
- existing `briefing_uses_provider_neutral_typed_routing_vocabulary` extended via
  `prompts::REQUIRED_MARKERS` with `bridge-steer`.

`live_turn.rs`
- `a_user_steer_reaches_a_live_worker_and_tells_the_parent` — a live worker +
  parent: `submit_input` succeeds, the worker's provider sees the wrapped text, the
  parent's runtime receives a `bridge-worker-steered-by-user` notice, and a
  `delegation.steered` event exists on the parent.
- `a_reported_worker_still_refuses_input` — the old rejection survives where it
  was actually right.
- `steering_a_worker_leaves_the_result_contract_alone` — after a steer,
  `worker_runtime.result_status` is still `pending` and `last_result` is still null.
- `an_orchestrator_steer_only_reaches_its_own_live_child` — `bridge-steer` at a
  foreign session id, a reported child, and a dead child are each refused with a
  reason fed back to the orchestrator; the legitimate case delivers.

### TypeScript — vitest

`src/conversation.test.ts`
- `folds a worker result into the panel that spawned it` — one delegation item for
  spawn + result, carrying both sets of data. (Replaces the existing
  "renders delegation spawn and result as delegation items" two-item assertion,
  which encoded the behavior this issue removes.)
- `leaves an orphan worker result visible` — a result with no spawn item still
  yields one item.
- `folds durable and live halves together` — spawn from `forestEntries`, result
  from the live stream, one item out.

`src/components/workerPanel.test.ts` (new)
- `projects the live facts a panel needs` — status, elapsed source, retry,
  progressSummary, waitingReason.
- `caps the mini-feed and keeps the newest lines` — 50 events in, ≤3 lines out,
  newest last.
- `ignores events belonging to other sessions`.
- `collapses repeated identical lines`.
- `returns null when the child session is unknown`.

`src/components/AgentConversation.test.tsx`
- `shows a live worker panel while the worker runs` — status label, progress line,
  feed line, expand + open-session affordances.
- `turns the same panel into the result card when the result lands`.
- `still shows a classified failure with its retry action after folding`.

`src/components/WorkerDetail.test.tsx`
- `offers a steering composer for a live worker`.
- `does not offer steering once the worker has reported`.
- `does not offer steering while the worker is checkpointing`.
- `shows no composer at all when the caller does not offer steering`.

`src/components/SteerComposer.test.tsx` (new, jsdom)
- `sends what the user typed and clears the box` — trimmed, through the injected
  handler.
- `refuses to send an empty steer`.
- `keeps the draft and shows why when the steer is refused` — the backend gate
  can refuse between render and submit.
- `explains itself instead of offering a box a worker cannot take`.

**Amended during implementation.** This section originally called for an
`src/App.test.tsx` case rendering the worker focus view end to end. `App.test.tsx`
only mounts `ChatModelControl`; the full `App` needs every Tauri command mocked,
which is out of proportion to what the case would prove. The composer the worker
view renders is `SteerComposer`, exported and tested directly above — same unit,
same assertions, no mock harness. The focus view's own wiring (banner replaced,
`workerSteerable` mirroring the backend gate) is covered by `tsc -b` plus the
manual path in §Manual below.

## Integration / Functional Tests

- `cargo test -p bridge-core` green — in particular the existing delegation,
  session_input, live_turn, prompt-lint, and protocol-mirror suites.
- `cargo test --workspace` green: asserts by omission that no protocol artifact,
  method registry, or handler mapping changed.
- `bun run test` green (vitest + sidecar + cargo).
- `bun run build` green (`tsc -b` is strict; unused imports fail the build).

## Smoke Tests

- `bun run check` — `tsc -b` + `cargo check --workspace`.
- `cargo test -p bridge-core session_input` and `... delegation::tests` pass in
  isolation.
- Prompt lint: `lint_required_markers(orchestrator::briefing())` returns empty, so
  the new marker cannot be silently dropped from a customized prompt.

## E2E Tests

N/A as automated coverage — the app is a Tauri desktop shell and this repo has no
driver harness for a live orchestrator + worker pair. Covered instead by the
`live_turn.rs` integration-style tests, which build a real core with a fake
adapter runtime and assert the delivered provider text and the parent notice.

## Manual / cURL Tests

No HTTP surface. Manual verification path in the app:

1. Open an orchestrator session in a workspace with a repo; delegate an
   implementation task ("Write scope: src/**" in the message to skip the approval
   card).
2. Watch the chat: the `Delegated to …` row must become a live panel with a
   WORKING pill, elapsed counter, and a moving mini-feed — no silent gap.
3. Click "Expand" → `WorkerDetail` overlay opens over the chat with the full feed.
4. Type into the overlay's composer ("also update the tests") → the worker's feed
   shows the guidance arriving; the orchestrator chat shows a "steered" chip.
5. Focus the worker session from the sidebar → composer present, banner gone.
6. Let the worker finish → the live panel becomes the result card in place; no
   second disconnected "Subagent finished" row.
7. Ask the orchestrator to redirect a running worker → it emits `bridge-steer`;
   the worker receives it and the chip appears.
