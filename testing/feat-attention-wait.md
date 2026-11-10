# Contract: sessions/wait and bridge exec --wait

Branch `feat/attention-wait`, stacked on `feat/attention-post`.
Implements #912, slice 8 of #904. Locked before implementation.

## What this changes

A script or an agent that starts work in a chat and needs to know when it
is done, or blocked on the person, has to follow the event stream itself and
decide what a finished turn looks like. Core already decides that for the
attention ledger; this lets a caller wait on it.

## Functional behavior

`sessions/wait { sessionId, until, timeoutMs?, afterSequence? } ->
{ state, matched, item? }`, protocol 1.30:

- `until` is a non-empty list of `finished`, `failed`, `needsYou`, `idle`.
- The pin: `afterSequence` when given, otherwise the ledger's newest sequence
  at the call. A `finished` or `failed` row counts only after the pin.
- A turn that starts after the call cannot satisfy it: without
  `afterSequence`, `finished` and `failed` count only if a turn was in flight
  at the call (`active_turn_id` set, or status `working` or `checkpointing`,
  the same test the input path uses). A caller that sends and then waits reads
  `latestSequence` first and passes it as `afterSequence`, which vouches that
  the turn it sent comes after the pin; that is the no-race form.
- `needsYou` is a state, not an event: it matches whenever the chat's newest
  row is an unresolved request, including one that was already open at the
  call.
- `idle` matches when no turn is in flight and nothing waits on the person.
  With `afterSequence` it also needs a `finished` or `failed` row after the
  pin first, so a just-sent turn that has not started yet is not idle.
- It returns at once when it already matches, and at once with
  `matched: false` when nothing in `until` can still happen (no turn in
  flight, no `afterSequence`, and neither `idle` nor `needsYou` can match).
- `state` is the matched state, or the chat's current one when unmatched;
  `item` is the matching ledger row when there is one.
- `timeoutMs` defaults to 10 minutes and is capped at 1 hour. A timeout is an
  error that says input may already have been sent, so read the chat before
  retrying.
- Waiting holds no database lock: the waiter follows the core event bus and
  re-reads briefly on each relevant event, and every 2 s in case the lossy
  bus dropped one. Many waiters do not block writers.
- An unknown or archived chat is an error.

Not taken from the issue's sketch, on purpose:
- `sessions/send_turn` does not gain a `wait` field: `afterSequence` gives
  the same no-race guarantee without changing `send_turn`'s result type for
  every client. `bridge exec` uses it that way.
- `turnId` pinning: turn ids are not on ledger rows; the sequence pin does
  the same job for the ledger's states.

`bridge exec --harness <id> --wait <states> "<prompt>"`:

- Creates the chat as today, reads `latestSequence`, sends, then calls
  `sessions/wait` with `afterSequence` instead of streaming events.
- Prints one JSON line with the result and exits 0 for `finished` (or
  `idle`), 3 for `needsYou`, 1 for `failed`, a timeout, or no match.
- Without `--wait`, exec behaves exactly as before.

## Unit tests (bridge-core)

- `attention_wait::tests::a_finished_row_after_the_pin_matches`
- `attention_wait::tests::a_turn_that_starts_after_the_call_does_not_match`
- `attention_wait::tests::an_open_request_matches_at_once`
- `attention_wait::tests::idle_matches_at_once_and_not_before_a_sent_turn_ends`
- `attention_wait::tests::nothing_left_to_wait_for_returns_unmatched`
- `attention_wait::tests::a_timeout_says_input_may_have_been_sent`
- `attention_wait::tests::waiters_hold_no_lock` (a waiter blocks while a
  second thread writes rows and reads freely, then wakes on its row)

## Integration / functional tests

- bridged dispatch: `sessions/wait` returns when another thread records a
  finished turn; refuses an empty `until` and unknown fields.
- `bridge` CLI: `--wait` parsing, and the exit code for each state.
- Protocol gates and a handshake test for 1.30.

## Smoke tests

- `cargo test -p bridge-core attention_wait`, `cargo test -p bridge-protocol`,
  `cargo test -p bridged`, `cargo test -p bridge-client`
- `bun run check`, `bun run build`, `bun run test`

## E2E / Manual

Live daemon from this branch: `bridge exec --method sessions/wait` on a
seeded chat with an open request returns `needsYou` at once; on an idle chat
waiting for `finished` returns unmatched at once. A real `bridge exec
--harness … --wait finished` run spends provider tokens, so it is listed for
the reviewer rather than run here.
