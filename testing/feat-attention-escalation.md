# Contract: escalate stranded worker notices to the person

Branch `feat/attention-escalation`, stacked on `feat/attention-desktop`.
Implements #907, slice 3 of #904. Locked before implementation.

## What this changes

A worker's result reaches its orchestrator as a row in
`queued_session_input`, delivered at the orchestrator's next idle boundary.
When the orchestrator has no live adapter (Bridge restarted, the adapter was
torn down), `drain_queued_input` leaves the row "unclaimed and quiet" so an
unattended sweep never spawns a provider (#252). Nobody is told, and the
result waits until the person happens to type into that chat. The live DB
has 16 such rows on idle orchestrators, 5 of them worker results.

This change tells the person within a bound and gives them one click to
deliver it.

## Functional behavior

Sweep (`attention::escalate_stranded`, called from the existing queued-input
sweep in `live_turn.rs` every 2 s, after its drains):

- A session is stranded when it has a `queued` row older than 30 s and newer
  than 24 h, has no live adapter, is not archived, exists, and has a visible
  kind.
- A stranded session with no open `escalation` row gets one: kind
  `escalation`, severity `actionable`, audience `human`, title
  "A result is waiting", body "<chat> · <Harness> stopped before it read N
  update(s)" with N the count of its stranded rows. Opening it supersedes the
  session's older unseen rows, like every other kind.
- "Open" means not resolved. A seen but unresolved escalation is not raised
  again: the person already knows.
- An open `escalation` row closes (`resolved_at`) once its session has no
  `queued` rows left, by any route: Deliver now, a send by the person, or a
  turn boundary.
- Rows older than 24 h are left alone, so upgrading does not raise a storm
  for months-old leftovers.
- The sweep never starts a provider process.
- The sweep publishes `attention/changed` when it opened or closed anything.

Deliver now (`sessions/deliver_queued_input { sessionId } -> { delivered }`):

- Refuses an archived session (existing error).
- Returns `delivered: false` when nothing is queued.
- Resumes the adapter through the same path a send by the person uses
  (`resume_for_send`) when it is not live, then calls `drain_queued_input`.
  A resumed chat is `ready` (#261), so the drain's idle gate lets it through.
- Exactly-once is the queue's existing claim: two concurrent calls deliver
  the row once.
- Resume failures return the existing "could not resume it" error.
- Runs on a blocking thread in the Tauri shell, like `start_chat`.

Desktop:

- An `escalation` row arrives without the 1 s hold (it is already 30 s old).
- Its toast reads "A result is waiting" with the row's body, and carries a
  **Deliver now** button next to the body. Deliver now calls
  `deliverQueuedInput`, dismisses the card on success, and shows the error
  through the app's existing error alert on failure. Clicking the body opens
  the chat, as for every toast.
- The OS banner is the usual one. On Linux it carries Open only, since there
  is no approval id.

Out of scope: `abandoned` rows. Boot already tells the person in the
transcript that those may not have reached the agent, and redelivering could
duplicate. They are left for #909's policy work.

## Unit tests (bridge-core)

- `attention::tests::stranded_session_escalates_once`
- `attention::tests::live_adapter_archived_hidden_and_old_rows_do_not_escalate`
- `attention::tests::seen_escalation_is_not_raised_again`
- `attention::tests::escalation_closes_when_the_queue_empties`
- `attention::tests::escalation_copy_counts_updates`
- `live_turn` tests (unit fixtures can attach a recording adapter but not
  launch a real one, so the real resume is exercised in the manual step):
  - `deliver_now_delivers_once`: with a live adapter, the queued row is
    sent once; a second call reports nothing queued.
  - `deliver_now_resumes_an_adapterless_chat`: with no adapter, the call
    attempts the resume a send would, surfaces its "could not resume it"
    error when the harness cannot launch, and keeps the row queued.

## Integration / functional tests

- bridged dispatch: `sessions/deliver_queued_input` with nothing queued
  returns `{"delivered": false}`; unknown fields are refused.
- Protocol gates as in the earlier slices (artifacts, naming, strictness,
  registry, signatures).

## Frontend tests

- `AttentionToasts.test.tsx`: a toast with an action renders the button and
  calls it; one without renders none.
- `attentionCopy.test.ts`: escalation copy, live and from the row.
- `App.attention.test.tsx`: an escalation row shows "A result is waiting"
  with Deliver now; clicking it calls `deliverQueuedInput` for that chat and
  dismisses the card.

## Smoke tests

- `cargo test -p bridge-core attention deliver_now`
- `cargo test -p bridge-protocol`
- `bun run check`, `bun run build`, `bun run test`

## E2E / Manual

Live daemon from this branch with its own data directory:

1. Insert a stranded `queued_session_input` row for a stopped chat (sqlite,
   with the daemon stopped), start the daemon, wait up to 32 s:
   `attention/list_attention` returns one `escalation` row.
2. Call `sessions/deliver_queued_input` for it: `{"delivered": true}`, and
   within one sweep the escalation row has `resolvedAt`.

## Review amendments (review 5817244323)

Locked before the fixes; one commit per finding, each with a regression test
that fails without it.

1. **Workers are never resumed from here.** A worker resumes only through its
   orchestrator, inside its pool reservation and with its sandbox and write
   scope. `deliver_queued_now` refuses a session with a parent (or depth > 0)
   before touching an adapter, and `escalate_stranded` never raises an
   escalation for one, so Deliver now is never offered for a worker. Tests:
   `deliver_now_refuses_a_worker_and_launches_nothing`,
   `workers_never_escalate`.
2. **A superseded escalation does not block its replacement.** The "one open
   escalation" guard counts only rows that are unresolved and not
   superseded, so a later row for the chat (a failed turn, say) cannot hide
   the remedy while input is still stranded. Test:
   `a_superseded_escalation_is_raised_again`.
3. **Only a handled queue dismisses the card.** `sessions/deliver_queued_input`
   returns `{ disposition }`: `delivered`, `nothingQueued` (someone else
   handled it), or `afterCurrentTurn` (the adapter is live and busy, so the
   ordinary boundary drain will deliver it). When the row is still queued on
   an idle chat after the attempt (a provider write failure, a busy
   lifecycle claim), the call returns an error naming that, the row stays
   queued, and the client keeps the card and shows the error. Tests:
   `deliver_now_reports_a_failed_write_as_an_error`, and the App test
   `keeps Deliver now when delivery fails`.
4. **Protocol compatibility.** With the `other` fallback added in #915, a
   1.24 client decodes `escalation` as `other` and shows nothing, so 1.25
   still accepts 1.24 clients; the handshake note says so. Test:
   `a_1_24_client_decodes_escalation_as_other` (protocol), plus the
   desktop feed's unknown-kind test.


## Review amendments (review 2)

Locked before the fix; it gets a regression test that fails without it.

5. **An escalation stays open while its input is being delivered.** Deliver
   now claims the row (`claiming`) before the provider write. A sweep in that
   window must not read the queue as empty: if the write fails, `release()`
   puts the row back to `queued`, and the card has to still be there. The
   close query counts `queued` and `claiming` rows as pending, as
   `runtime_budget` already does. Test:
   `escalation_stays_open_while_delivery_is_claiming`.
