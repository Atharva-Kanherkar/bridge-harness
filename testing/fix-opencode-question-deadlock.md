# fix/opencode-question-deadlock — Test Contract

GitHub issue #282: an OpenCode session using the built-in `question` tool
deadlocks. OpenCode raises a dedicated `question.asked` SSE event, answered
over `POST /question/{requestID}/reply` — a different channel than
`permission.v2.asked`/`/permission/{id}/reply`. Bridge did not understand
`question.asked` at all (it fell through to `provider.unknown`, so the
session never flipped to `waiting` and the tool row just spun), and even if
it had, the typed reply had nowhere to go: OpenCode advertises no active-turn
steering, so a typed answer queued for a turn boundary that a still-open
question can never reach. Circular wait; the only way out was interrupting
the turn.

## Root causes

1. `normalize_opencode_message_with_state` (`bridge-core/src/agent.rs`) has no
   arm for `question.asked`.
2. `OpenCodeAdapter::respond` always posts to `/permission/{request_id}/reply`
   — the wrong endpoint for a question, and shaped for a decision
   (`accept`/`decline`), not a text/option answer.
3. `submit_input` routes by `supports_active_turn_steering()`, which OpenCode
   leaves `false`; a typed reply while a question is open is queued for a
   turn boundary the open question is itself blocking.

## Functional Behavior

- `question.asked` normalizes to `approval.requested` with
  `data.requestMethod = "opencode.question"`, `data.requestId`, and
  `data.questions` (the raw question list), plus a human-readable title/text.
  It does **not** end with `requestApproval`, so the existing bypass-policy
  gate in `live_turn.rs` (`is_approval_request`) correctly refuses to
  auto-grant it — a question needs an answer, not a decision.
- The generic `approval.requested` handling already marks the session/worker
  `waiting`, so a pending question stops implying progress without any new
  branch there.
- `submit_input` on a session with an unresolved `opencode.question` approval
  delivers the typed text as that question's answer (`POST
  /question/{requestID}/reply`) instead of queuing it, marks the approval
  resolved, and flips the session back out of `waiting` — mirroring exactly
  what `resolve_approval` already does for a card-driven resolution.
- `resolve_approval` refuses `accept`/`acceptForSession` against a
  question-marked approval (there is no text to answer with from a bare
  decision) and maps `decline`/`cancel` to `POST /question/{requestID}/reject`
  instead of the permission-reply endpoint.
- Sessions/providers with no pending question are unaffected: routing falls
  through to the existing new-turn/steer/queue table exactly as before.

## Unit Tests

- `agent.rs`: `question.asked` → `approval.requested`, `requestMethod`,
  `requestId`, `questions`, title/text all populated; unknown providers still
  fall through to `provider.unknown` (`preserves_unknown_provider_event`
  keeps passing).
- `live_turn.rs`: a session with a pending `opencode.question` approval
  answers it on `submit_input` (steered disposition, adapter's
  `answer_question` called with the right request id and answer shape, no
  queue row created, approval resolved, session back to `working`); a session
  with no pending question still queues/steers/starts exactly as before.
- `api.rs` (or `live_turn.rs` resolve-approval tests, wherever the existing
  suite lives): `accept` against a question-marked approval is rejected with
  a clear error; `decline` maps to the reject endpoint.

## Integration / Smoke

- `cargo test -p bridge-core` green.
- `cargo build` (or the project's usual build gate) green.

## E2E

N/A — reproducing the live OpenCode deadlock needs a real `opencode-ai`
sidecar and a stale-workspace prompt; covered by the unit tests above plus
manual reasoning against the pinned SDK's `types.gen.d.ts` /
`sdk.gen.js` (`Question.reply`/`reject`, `/question/{requestID}/reply`
posting `{"answers": [...]}`).

## Local checkpoint

`cargo test -p bridge-core agent::` and `cargo test -p bridge-core
live_turn::` targeted runs, plus a full `cargo test -p bridge-core` before
opening the PR.
