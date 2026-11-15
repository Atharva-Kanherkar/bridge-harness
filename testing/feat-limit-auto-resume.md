# Test contract: usage-limit auto-resume and backup model

Branch `feat/limit-auto-resume`. One setting, "When a usage limit stops a chat": **Do nothing** (default), **Resume when the limit resets**, or **Switch to a backup model** (harness, model, effort).

## Detection (eval: `provider_limit::tests::eval_recorded_limit_frames`)

Corpus: `testing/fixtures/limit-recovery/frames.json`. These are real error frames from the live database, labeled, with `now` pinned.

| Gate | Threshold |
| --- | --- |
| Recall on limit frames | 22/22 |
| False positives on other errors (auth, billing, 401s with hex ids, network, flagged content, context limit) | 0 |
| Kind (usage vs capacity) | all correct |
| Reset recovered to the minute, in the frame's named zone, else the machine zone | 17/17 |

Before this branch, the detector missed every Claude "You've hit your … limit · resets 7:50pm (Asia/Tokyo)" frame, ignored the named zone, and could not parse "Sep 7th, 2026 11:35 AM".

## Fuzz (proptest; `PROPTEST_CASES` overrides the case counts)

- `provider_limit` fuzz: frame-shaped noise and arbitrary Unicode never panic, and any reset is strictly after now and within 30 days. Named-zone resets read back on that zone's clock, within 24 h.
- `limit_recovery::plan_invariants`: off and capacity never schedule. Nothing is scheduled in the past. Resume never outruns reset plus grace, or the probe cap. Only backup mode switches, and only to a different model. Attempts stay within 1..=6.
- `limit_recovery::store_and_tick_interleavings`: random interleavings of limits, user messages, cancels, resume-now and ticks against the real store. Sends never exceed scheduled rows, and off sends nothing.

## Unit (`limit_recovery::tests`)

Planner, guard (deleted, archived, catch-up window, already running, model changed), settings validation, observe (off, workers, duplicate frames, give-up), tick (wait, act once, switch then send, failed send recorded and not retried), crash recovery from `acting`, resume-now, streak reset by a person's message, and concurrent ticks acting exactly once.

## End to end (`src-tauri/bridged/tests/limit_recovery.rs`)

Real daemon, real Claude sidecar, fake SDK (`testing/fixtures/limit-recovery-e2e/`):

1. J1: resume waits for the stated reset plus grace, then continues on the same model, exactly once.
2. J2: backup mode refuses a missing backup, an unlisted model and an unsupported effort at save. It then switches model and effort immediately and continues. A backup that also hits its limit waits for its own reset.
3. J3: a person's message cancels, the banner's Cancel cancels, Resume now acts at once, and a second Resume now is refused.
4. J4: off by default, so nothing happens.
5. J5: a pending row survives a daemon restart and fires after it.

## Frontend (Vitest)

- `LimitRecoverySection`: off by default; resume saves in one choice. Backup mode stays a draft until a model is picked. Unavailable harnesses are hidden, only storable efforts are offered, and a refused save is shown.
- `LimitRecoveryBanner` and `limitRecovery.ts`: countdown text, labels, Resume now and Cancel call the API, it rereads on status change, failures and give-ups are explained without actions, and finished rows are hidden.

## Mutation (cargo-mutants on `limit_recovery.rs` and `provider_limit.rs`)

The live wiring that only the e2e exercises is excluded: `LiveActions`, the maintenance loop, `tick_interval`, and the `caffeinate` spawn. Every surviving mutant is killed by a new test or explained in the PR.
