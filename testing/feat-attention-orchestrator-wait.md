# Contract: orchestrator wait conditions (bridge-wait)

Branch `feat/attention-orchestrator-wait`, stacked on
`feat/attention-macos-actions`. Implements #911, slice 7 of #904. Locked
before implementation. The orchestrator decides when to use this, so it ships
with a live eval (approved by the person, with its model spend).

## What this changes

Every worker result is a separate orchestrator model turn today: three
parallel workers cost three turns, each reading one result while the others
still run, and nothing joins them or times them out. With `bridge-wait` the
orchestrator says "wake me when they are done", ends its turn, and gets one
turn with every result.

## Functional behavior

Block, parsed beside `bridge-peek` in `delegation.rs`:

```bridge-wait
{"workers": "all", "until": "all_done", "timeoutSeconds": 900, "breakOn": ["failed", "needs_you"]}
```

- `workers`: `"all"` or a non-empty list of session ids. `until`:
  `all_done` (default) or `any_done`. `timeoutSeconds`: 30 to 3600, default
  900. `breakOn`: any of `failed`, `needs_you`; default both. Unknown fields,
  a second block, or bad values are refused, like the other blocks.
- The block is stripped from the prose the person reads and held to the
  turn's end, like a peek.

Arming (at the turn's end):

- `"all"` resolves to the orchestrator's live workers at that moment
  (`worker_runtime.parent_session_id` = the orchestrator and `result_status`
  = `pending`). A listed id must be one of those. Anything else refuses the
  whole wait with `bridge-wait-rejected` (to the model and as a transcript
  row, the path a refused steer takes). `"all"` with no live workers is
  refused too: there is nothing to wait for.
- One wait per orchestrator. A new one replaces an armed one and keeps what
  it already holds.
- A worker result already queued for a listed worker but not yet delivered
  moves into the wait, so a result that raced the orchestrator's own turn is
  not delivered on its own.
- A successful arm sends nothing back: the point is to stop spending turns.
- Stored durably (migration 72): `orchestrator_waits` (one row per
  orchestrator: workers, until, breakOn, deadline) and
  `orchestrator_wait_results` (held result notices). Held results live there
  rather than in `queued_session_input`, so the queue's delivery rules are
  untouched; the effect the issue asks for is the same.

While armed:

- A `bridge-worker-result` for a listed worker is held instead of queued.
  The transcript still records the result for the person (`held: true`).
- Results for workers not in the wait are delivered as today.
- An essential routing notice (launch declined or failed, a worker stopped)
  breaks the wait, unless `breakOn` is explicitly empty, and travels inside
  the delivery instead of as a turn of its own.
- With `needs_you` in `breakOn`, a listed worker blocked on approval breaks
  the wait.
- Informational notices are not delivered while armed: they are what the
  wait exists to save, the person sees approvals directly, and the
  orchestrator was already told never to wait on them.
- Actionable notices are delivered as today.

Firing, checked when a result is held and on the 2 s sweep:

- `failed` in `breakOn` and a held result is failed, blocked or
  protocol-invalid: fires with reason `failed`.
- `all_done`: every listed worker has a held result or is no longer live.
  `any_done`: one held result.
- The deadline passed: reason `timeout`.
- Firing enqueues one `bridge-worker-results` envelope for the orchestrator
  in the same transaction that clears the wait:
  - `reason`;
  - `results`: each held `bridge-worker-result`, unchanged;
  - `stillRunning`: listed workers with no result;
  - the `fleet` digest;
  - the breaking `notice`, if any;
  - an `instruction` not to claim completion while anything is still running.

  The queue then delivers it at the next idle boundary, as it does today.
- Without an armed wait, delivery is exactly as today.
- A daemon restart keeps an armed wait and its held results; the sweep fires
  an overdue one after restart.

Prompt: a "Waiting for several workers (bridge-wait)" section in
`delegation::protocol`, next to `bridge-peek`: after a parallel fan-out,
emit one `bridge-wait` and stop; do not claim the task is done until the
results arrive, and report what is still running when a wait times out.

## Unit tests (bridge-core)

- `delegation`: parser accepts the full and the minimal forms and refuses
  unknown fields, bad `until`, an out-of-range timeout, an empty list and two
  blocks; `strip_wait` keeps the prose.
- `orchestrator_wait`:
  - arming resolves `"all"` and refuses a worker that is not the caller's,
    and `"all"` with nothing running;
  - three results under `all_done` are held and fire exactly one envelope
    with all three;
  - a failed result fires at once with the results so far;
  - `any_done` fires on the first;
  - the timeout fires with `stillRunning`;
  - an essential notice fires with the notice inside;
  - a result for an unlisted worker is queued as today;
  - a re-arm keeps held results;
  - a queued result is adopted at arm time;
  - the state survives a reopened database (restart).
- `live_turn`: a turn ending with a `bridge-wait` arms it and strips the
  block; an invalid block is refused back to the orchestrator.

## Eval (live, opt-in)

`orchestrator_wait_eval.rs`, `#[ignore]`, run with
`cargo test -p bridge-core orchestrator_wait_eval -- --ignored`. It drives
real Claude, with the production orchestrator briefing and delegation
protocol as instructions, from a fresh empty cwd. Cases are in
`testing/evals/orchestrator-wait/cases.json`.

- `fan-out` cases: a task with three independent parts. Score from actions:
  the first turn emits two or more `bridge-delegate` blocks (production
  parser) and exactly one valid `bridge-wait` (production parser), and its
  prose does not claim the work is finished.
- `timeout-digest` case: the orchestrator is handed a `bridge-worker-results`
  envelope with two results and one worker still running (reason `timeout`).
  Pass: the reply does not claim the task is complete and names the
  still-running worker.
- Control: the fan-out cases rerun with the `bridge-wait` section removed
  from the protocol must not emit `bridge-wait`, which shows the guidance is
  what produces it.
- Gate: every fan-out case and the digest case pass, the control emits no
  wait. The run prints each case's blocks and reply for review.

## Smoke tests

- `cargo test -p bridge-core orchestrator_wait`, `cargo test -p bridge-core delegation`
- `bun run check`, `bun run build`, `bun run test`

## E2E / Manual

A real three-worker run costs three worker sessions; it is left to the
reviewer: ask an orchestrator for three independent read-only research
tasks, check it emits `bridge-wait`, and that it gets one turn with all three
results.
