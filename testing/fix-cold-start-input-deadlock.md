# fix/cold-start-input-deadlock — Test Contract

Fixes #261. A new Codex or OpenCode chat cannot receive its first message: it is
queued for a phase boundary that can never arrive, and the queue can never drain.

## The defect, precisely

`start_chat` marks a session `status='working'` with `active_turn_id=NULL` the
moment its provider process is up. No turn has been started — for Codex that is
literally true, `codex_adapter::start` does `initialize` + `thread/start` and
nothing else.

`turn_is_active` then reads `status='working'` as *a turn is in flight*, so
`session_input::route` returns `Queue` for any provider that does not advertise
active-turn steering (`ClaudeRuntime` is the only adapter that does).
`drain_queued_input`'s idle gate is the mirror image of the same predicate, and in
production only the `turn.completed` frame handler ever writes `status='ready'` —
which cannot fire, because no turn was started.

Note that Bridge already encodes the invariant this violates, from the other
direction: boot reconciliation (`store.rs` ~85) clears `active_turn_id` for any
session whose status is `ready`/`stopped`/`failed`/`completed`/`cancelled`,
because "a session in a terminal state has no active turn". A started-but-idle
session claiming `working` with no turn is the same inconsistency inverted.

## Functional Behavior

### 1. Starting a session does not claim a turn

- `start_chat` leaves a freshly started session `status='ready'`, not `'working'`.
  `'ready'` already means "up and idle" everywhere else: it is in the frontend's
  `liveStatuses` (so a send does not re-start the session), Mission Control renders
  it `READY`, and boot reconciliation already treats it as turn-free.
- `turn_is_active` is therefore false for a started, idle session, and the first
  message routes to `NewTurn` on **every** provider — steering-capable or not.
- The deliberate pessimism in `turn_is_active` is **kept**: `status='working'`
  still counts as a turn in flight, because the window between
  `deliver_prepared_input` writing a turn and the provider echoing `turn.started`
  is real and a second `turn/start` must not land in it. This change removes a
  false claim of that state, it does not weaken the guard.
- No status vocabulary is added. `Starting` exists in the enum but is absent from
  `liveStatuses`, so using it here would make the frontend re-start the session on
  every send.

### 2. The drain does not spin against a dead provider — *already fixed; guarded here*

**Corrected during implementation.** This section originally claimed the retry
spin as a live second bug, on the strength of 2,254 `session.input.delivery_failed`
rows measured in a 42-minute window (2026-08-21 06:07–06:49 UTC). That was a
misattribution: `5be7f5ac` ("a send resumes a dead adapter, and a parked queue
stops spinning", #252) landed at 06:37 UTC and already makes
`drain_queued_input` return early — without claiming the row and without writing
a ledger event — when the session has no live adapter. The rows I measured were
the ledger of a *pre-fix build* that was still running; the app died at 06:49 and
nothing has spun since.

So there is nothing to fix here. What this contract keeps is a **regression
guard**, because the behavior is easy to lose and expensive when lost:

- With no live adapter, `drain_queued_input` is false, the row stays `queued`
  (never `claiming`, never released), and no reason row is written.
- Nothing is lost by skipping: the sweep delivers once the provider is back.

### 3. Input for a conversation that ended is retired, not left silently queued

- A queued row whose session is in a terminal state (`stopped`, `failed`,
  `completed`, `cancelled`) is retired as `abandoned` and surfaced in that
  session's transcript, reusing the existing `emit_local_assistant` copy path that
  already handles rows stranded mid-write across a restart.
- Confirmed still missing: `discard_for_session` is called from exactly one place
  (the `/clear` path, `live_turn.rs` ~7152, which resets a session to `idle`). A
  session that merely *stops* — process exit, app death — leaves its queued rows
  behind. Five such rows are sitting in a real database now, quiet since #252 but
  never delivered and invisible.
- Rationale, and the same one `session_input::discard_for_session` already states:
  the conversation the follow-up belonged to is gone. Delivering an hour-old
  message into a session the user has since restarted is a surprise, and leaving
  it invisible in SQLite forever is worse.
- A queued row for a session that is merely *between* providers (`idle`, `ready`,
  `warm`, `resuming`) is **not** retired — that session can still come back.

## Unit Tests

### Rust — `bridge-core`

`live_turn.rs` — `submit_input_tests`
- `a_cold_sessions_first_message_starts_a_turn_instead_of_queueing_forever` — the
  #261 reproduction. A session in the state `start_chat` leaves behind, with a
  provider that cannot steer: the disposition is `StartedNewTurn`, the text
  reaches the provider, and nothing is queued.
- `a_started_session_is_idle_until_a_turn_is_actually_submitted` — after
  `start_chat`-shaped state, `turn_is_active` is false; after
  `deliver_prepared_input`, it is true.
- `a_turn_bridge_has_written_but_the_provider_has_not_echoed_still_blocks_a_second`
  — the pessimism this change preserves: `status='working'` with no
  `active_turn_id` (as `deliver_prepared_input` leaves it) still routes a second
  message to `Queue`/`Steer`, never a second `NewTurn`.
- `the_drain_does_not_claim_or_log_when_the_provider_is_gone` — no live adapter:
  `drain_queued_input` is false, the row is still `queued` (not `claiming`, not
  released), and **no** `session.input.delivery_failed` row was written.
- `a_queued_row_survives_a_provider_outage_and_lands_when_it_returns` — skipped
  while dead, delivered once an adapter is attached.
- `input_for_a_stopped_session_is_retired_and_said_out_loud` — a terminal session's
  queued row becomes `abandoned`, a `session.input.abandoned` ledger row exists,
  and the session's transcript names the text that was not sent.
- `input_for_a_session_between_providers_is_kept` — `idle`/`ready`/`warm` rows are
  left `queued`.

`session_input.rs`
- existing suite must stay green unchanged: the queue's exactly-once, ordering,
  release, and recovery semantics are not touched by this fix.

## Integration / Functional Tests

- `cargo test --workspace` green. In particular the existing
  `a_provider_that_cannot_steer_gets_a_durable_queue_not_a_second_turn` and
  `a_dead_adapter_parks_the_queue_instead_of_spinning_on_it` tests must be
  re-read, not just re-run: the first asserts the behavior this fix *keeps* (a
  real in-flight turn queues), and the second names the exact spin this fix
  removes, so its fixture and assertions need to match the new path.
- `bun run test` green — no frontend change is expected in this fix, so a
  frontend diff would itself be a finding.
- `bun run build` and `bun run check` green.

## Smoke Tests

- Reproduction, before and after, run against `origin/main` in a throwaway
  worktree as well as this branch, so the fix is shown to change the behavior
  rather than merely to pass.
- `cargo clippy -p bridge-core --all-targets` — warning count in
  `live_turn.rs` must not exceed `origin/main`'s (20 at the time of writing).

## E2E Tests

N/A as automated — no Tauri driver harness in this repo.

Verified manually instead, in a release bundle installed to `~/Applications`:
start a **new Codex** orchestrator, send one message, and confirm a turn starts
rather than the composer showing "1 follow-up queued — sent when this step
finishes". That is the exact user-visible failure in #261.

## Manual / cURL Tests

No HTTP surface. Against a live daemon:

1. Confirm no session is left in the deadlocked shape:
   `SELECT id,status,active_turn_id FROM sessions WHERE status='working' AND active_turn_id IS NULL;`
2. Confirm the spin is gone — after leaving a stranded row for several minutes,
   `SELECT COUNT(*) FROM events WHERE kind='session.input.delivery_failed';`
   must not grow.
3. The pre-fix workaround (`sessions/send_turn` via the `bridge` CLI) should no
   longer be necessary; `sessions/submit_input` on a cold session must start a turn.
