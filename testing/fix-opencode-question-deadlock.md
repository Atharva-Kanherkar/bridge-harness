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

## Review round 1 — four blocking findings

The first version of this fix answered a pending question by copying the
composer text straight to the adapter and into history, outside every other
input path's boundary, with no protection against two resolvers racing the
same request, no restriction to the one-question case OpenCode's `answers`
shape actually supports, and no way to stop retrying a request id once it
could no longer succeed. Each is now closed:

1. **Secret interception bypassed.** The answer skipped `prepare_input`
   entirely (correctly — an answer is literal text, not a slash command) but
   that also skipped secret interception and the credential broker, the one
   boundary every other input path crosses. `answer_pending_question` now
   calls `secret_interception::intercept` and `credential_broker.register`
   itself before the text reaches the adapter call or durable history.
2. **Check-then-act race.** Reading "is a question pending" and writing its
   resolution were unguarded separate steps; a second submission, or this
   path racing a card's Decline through `resolve_approval`, could both see it
   unresolved and both call the adapter. Both paths now hold
   `BridgeCore::claim_session_lifecycle(session_id, "question resolution")`
   across their whole critical section, released by `Drop` on every return.
3. **Multi-question fan-out.** A single composer string was copied into every
   positional slot of OpenCode's `answers` array, so distinct questions in one
   request got the same unintended answer. `answer_pending_question` now only
   intercepts a request with exactly one question; anything else falls
   through to ordinary routing.
4. **Orphaned rows outlive the process that can answer them.** An unresolved
   question survived adapter death or session stop, so a resumed session (a
   fresh process, an unrelated request-id namespace) retried the same dead
   request id and failed the same way forever. Now: a failed delivery voids
   the stale request and falls through instead of erroring (the text still
   reaches the session as an ordinary message); `stop_session` and the
   provider-error path void any request still open when the session goes
   away; and `question.replied`/`question.rejected` normalize to
   `question.settled` so a question answered through any other channel — a
   decline, a different client on the same OpenCode session — still resolves
   Bridge's own record instead of leaving it stuck open.

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
  `requestId`, `questions`, title/text all populated; `question.replied` /
  `question.rejected` → `question.settled` carrying the settling `requestId`;
  unknown providers still fall through to `provider.unknown`
  (`preserves_unknown_provider_event` keeps passing).
- `live_turn.rs`: a session with a pending `opencode.question` approval
  answers it on `submit_input` (steered disposition, adapter's
  `answer_question` called with the right request id and answer shape, no
  queue row created, approval resolved, session back to `working`); a session
  with no pending question still queues/steers/starts exactly as before.
- `live_turn.rs` (review round 1): a pasted credential in a question answer is
  intercepted before it reaches the adapter or durable history; a second
  resolver is refused while the claim is held; a two-question request falls
  through to ordinary routing instead of answering both the same way; a
  failed delivery voids the stale request and lets the text fall through
  instead of erroring, and a following submission is not stuck retrying the
  same dead id; `void_orphaned_questions` resolves a pending row directly; a
  `question.settled` echo from the provider unblocks a waiting session even
  when this process never itself answered it.
- `api.rs` / `live_turn.rs` resolve-approval tests: `accept` against a
  question-marked approval is rejected with a clear error; `decline` maps to
  the reject endpoint; a card decision against a question another path is
  already resolving is refused (the claim is shared across both entry
  points).

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
live_turn::` targeted runs, plus a full `cargo test -p bridge-core` (1311
passed, 0 failed, up from 1303 before review round 1) and `cargo clippy -p
bridge-core --lib --no-deps` (53 pre-existing warnings, none in a touched
file) before pushing.
