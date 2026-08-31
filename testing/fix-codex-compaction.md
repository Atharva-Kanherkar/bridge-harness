# fix-codex-compaction — Test Contract

Locked before implementation. The branch started from freshly fetched
`origin/main` at `d0edbc353d114b8cc8ee39c495a77b616ddced4c` in Bridge's isolated worker
worktree.

## Functional Behavior

### 1. Bounded, useful Codex activity

- Documented Codex app-server notifications that are internal bookkeeping or
  redundant transport progress do not become durable `provider.unknown`
  session-forest entries. A genuinely unknown future notification remains a
  collapsed, inspectable provider event for diagnostics.
- Assistant text, reasoning, command output, file changes, tool lifecycle, and
  the latest MCP tool progress remain observable.
- Repeated progress snapshots for the same session/item replace the previous
  snapshot rather than concatenate duplicate text. Append-only deltas remain
  append-only.
- Transient events use their semantic session/kind/item identity; their wire
  `id = 0` is never treated as a globally unique durable id.
- The pre-render queue, retained event count, individual merged text, and total
  retained text are bounded. A burst produces at most one React state update
  per scheduled flush, not one update per provider frame.

### 2. Compaction protocol is never conversation content

- While an internal checkpoint turn is active, its streamed and completed
  content remains suppressed before persistence and publication.
- As defense in depth, a checkpoint protocol envelope accidentally present in
  live events or old durable history is omitted by conversation projection.
  This includes plain JSON and a fenced JSON object carrying the complete
  Bridge compaction signature (`schemaVersion`, `summary`, `decisions`,
  `filesTouched`, `sourceAgent`, `firstRetainedEntryId`, `tokensBefore`, and
  `reason`). Ordinary JSON answers remain visible.

### 3. Model-switch compaction has one state boundary

- A `before_downgrade` summary belongs only to the outgoing provider/session.
  Invalid output, timeout, exit, or delivery failure settles that request before
  the old runtime is torn down.
- A failed model-switch summary does not start generic compaction reconstruction
  that can race and append old-session state after the new model selection
  commits. The switch instead uses the existing bounded mechanical projection.
- The model change still commits after summary failure, clears the old provider
  session, and the next start uses only the selected harness/model plus durable
  projected history.
- Terminal entries from older compactions cannot settle the current switch
  request.

### 4. Actionable failure and retry UX

- Every new `compaction.failed` entry records a stable failure class, a safe
  human-readable message, the trigger, retry eligibility, and a recovery action
  while retaining the technical reason for audit/debugging.
- Timeout/exit/unavailable-provider failures explain that original history is
  intact and can be retried. Invalid/unverifiable checkpoint output explains
  that Bridge rejected unsafe summary data and retained the conversation.
- Model-switch failures explain that switching continued with stored history;
  they do not imply that context was discarded.
- A failed compaction card shows the human message and recovery guidance. When
  retryable it offers one explicit retry action, disables it while running, and
  reports a retry error without duplicating requests.
- If manual checkpoint delivery fails after a request was appended, Bridge
  records a classified terminal failure and clears pending state so a later
  retry can start.

## Unit Tests

- `agent::codex_internal_notifications_do_not_become_unknown_events` — known
  high-volume bookkeeping methods normalize to no conversation event; a future
  unknown method remains `provider.unknown`.
- `appendAgentEventBatch` regressions — `id = 0` transients from different
  items survive; additive deltas merge; progress snapshots replace; interleaved
  progress coalesces; queue/count/text budgets hold under a large burst.
- `conversation` regressions — plain and fenced internal checkpoint envelopes
  are suppressed from live and durable projection; ordinary JSON is retained.
- `compaction_controller` failure classification cases — timeout/exit,
  unavailable delivery, invalid/schema/evidence output, and generic failures
  produce stable message/retry/recovery fields.
- `live_turn` model-switch recovery policy — `before_downgrade` failure cannot
  schedule reconstruction; normal pressure/manual failures retain recovery.
- Manual delivery failure regression — pending state is cleared with one
  classified `compaction.failed`, and a subsequent begin is allowed.
- `AgentConversation` failure card regressions — human copy renders, technical
  protocol text is not the primary message, retry invokes once, busy state
  suppresses duplicate clicks, and rejected retry is shown inline.

## Integration / Functional Tests

- Codex normalization → event retention → conversation reduction preserves one
  useful tool row under a large sequence of duplicate progress notifications
  without durable unknown-event growth.
- Model-switch failure → teardown → commit leaves no pending compaction or
  post-switch reconstructed boundary and reports carried mechanical context.
- Frontend retry calls the existing compaction command, refreshes the forest,
  and leaves the card actionable after a transient failure.

## Smoke Tests

- `bun run build` passes.
- `bun run test` passes (frontend, Claude sidecar, and full Rust workspace).
- Fresh-base invariant remains true: the branch merge-base with `origin/main`
  is `d0edbc353d114b8cc8ee39c495a77b616ddced4c`.

## E2E Tests

N/A — this repository has no automated Tauri desktop E2E harness. The provider
boundary is covered with deterministic Rust frames and the UI with Vitest/jsdom.

## Manual / cURL Tests

1. Run a Codex chat that performs a verbose MCP/tool operation. Tool name,
   running state, latest progress, and completion remain visible; the UI stays
   responsive and does not accumulate raw provider rows.
2. Switch a long Codex chat to another supported model. No checkpoint JSON,
   fenced envelope, or maintenance reasoning appears. A successful summary is
   carried; a failed summary clearly says the switch continued with stored
   history.
3. Force a checkpoint timeout or invalid response. The failure card explains
   what happened, states that original history remains, and offers retry.
4. Retry after the provider is available. Exactly one new checkpoint request is
   created and the card reports progress/errors without hanging the UI.
5. No cURL case applies: compaction is a local Tauri/daemon protocol command,
   not an HTTP endpoint.
