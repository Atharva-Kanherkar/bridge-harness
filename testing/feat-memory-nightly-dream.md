# feat/memory-nightly-dream — Test Contract

Issue #1149, the last part of epic #1144. Locked before implementation. It
reuses `memory_consolidation(_live)`: a dream is a consolidation run with a
nightly cadence, proposal review, a report and undo.

## Functional Behavior

### Cadence

- Nightly cleanup is **on by default**. A scope with no settings row reads mode
  `propose`, and `off` stays available. A run still needs a resolved memory
  model. With none, the claim is cancelled with `no_memory_model`.
- `dream_hour` is an account setting, 0–23, default 3. The *slot* is the
  latest local instant at `dream_hour:00` that is at or before now.
- An automatic dream is queued (trigger `nightly`) on a maintenance tick when
  all of these hold:
  - the mode is on, a memory model resolves, and no dream is queued or
    running;
  - the newest dream (any trigger, any status) started before the slot and at
    least 20 hours ago, **or** this is the first tick since launch and the
    newest dream started more than 24 hours ago. An install that has never
    dreamed meets this at once;
  - something changed since the newest dream settled. That means a
    `memory_records` row in the scope with a later `updated_at`, or a later
    retention sweep. With no dream yet, it means the scope holds at least one
    active or waiting record.
- No automatic dream starts less than 20 hours after the last one.
- `enqueue_when_full` stays as an extra trigger (`full`). It keeps its
  debounce and also obeys the 20-hour floor and the nothing-changed skip.
- A finished turn **no longer** queues consolidation.
- "Dream now" (`memory/start_dream`):
  - It queues a `manual` run due at once, whatever the hour, floor, changes or
    mode.
  - If a run is already queued or running, it returns that run instead of
    adding a second.
  - With no memory model, it is refused.
  - A disabled mode cancels queued `nightly` and `full` runs at claim time
    (`consolidation_disabled`), never `manual` ones.

### Input

- Every `active` and `proposed` record in the scope, read up to 600.
- Each record is shown as JSON with these fields:
  - an alias `r1..rN` that is unique within the dream;
  - `state`, `kind`, and `pin` (provenance `user_explicit`);
  - `created` and `lastUsed` dates, and `confidence`;
  - `expiresAt`, `group`, and `replaces` (an update proposal's target alias);
  - `body`, clipped to 400 characters.

  The alias map stays server-side.
- Records are clustered:
  - by the same `conflict_group`;
  - an update proposal with its target;
  - by lexical overlap: at least 2 shared `memory_similarity::terms`
    covering at least a third of the smaller record's terms.
- Clusters are packed in order into batches of at most 60 records and 16,000
  payload characters. A cluster splits only when it is larger than a batch.
- A dream runs at most 6 batches, one bounded hidden turn each. Records beyond
  them are **deferred** to the next dream and counted.
- The prompt has three parts, in order:
  1. Bridge framing: today's date, and whether proposals may be approved.
  2. The untrusted records block.
  3. The rules, with the vocabulary for the mode.

### Operations and guards

- The answer is the last fenced `bridge-memory-operations` JSON array, with at
  most 20 elements per batch. Targets are aliases **from this batch**. A raw
  id or an alias from another batch is refused.
- Existing operations: `merge`, `correct`, `expire`, `group`, `retire`, `keep`.
  New: `approve` and `reject`. An unknown field or operation is invalid.
- `merge`:
  - It takes 2 or more targets that are all active, or all waiting proposals
    without `replaces`. Mixed targets are refused.
  - Merging actives writes one active successor, as before.
  - Merging proposals writes one waiting proposal that supersedes them all,
    so the queue shrinks. Its confidence is the lowest of its sources.
- `correct` takes 1 active target, a `body` and a `basis`, another alias in
  the batch.
- **Grounding:** a `merge` or `correct` body is refused (`ungrounded`) when it
  has a content term (`memory_similarity::terms`, trailing `s` folded) found
  in none of its sources. For `correct`, the sources are the target and the
  basis. The framing words `user`, `users`, `prefers`, `prefer`, `uses`, `use`
  and `also` are exempt.
- `retire` is now a **retirement**: status `expired`, with the interval closed
  now. It can be restored from Retired, so it no longer needs `allowRemoval`.
  That field stays on the wire, deprecated and ignored.
- `approve` exists only in Automatic mode (extraction mode `auto_apply`). It
  takes 1 waiting proposal. It is refused when any of these holds:
  - the body has a transient marker;
  - the body matches a forgotten claim;
  - the body near-duplicates an active memory;
  - the body conflicts with an active memory it does not replace;
  - the budget is full and the proposal replaces nothing.

  Approving an update proposal swaps it for its target, as before.
- `reject` takes 1 waiting proposal and a `reason` of 1–200 characters.
  - In Review-first mode it also needs `duplicateOf`: an alias of another
    record in the batch that shares at least 2 terms with it. Otherwise it is
    refused.
  - In Automatic mode, `duplicateOf` is optional.
- Review-first mode (`propose` or `remember`) leaves every other proposal for
  the user. Merges, corrections, expiries, groups and retirements of active
  memories run in both modes.
- Pins, and records restored by an undo (*held*), accept only `keep`.
- A record is the target of at most one applied operation per dream. A later
  operation on it is refused.
- Caps per dream:
  - Spend of $0.50 (500,000 µUSD), checked before each batch. Once it is
    reached, no further batch starts, and the remaining records are deferred.
  - 60 applied operations. Operations beyond that are refused (`over_cap`).
- Every refusal is counted by reason, and the rest of the batch still applies.

### Report and run

- Each dream is one `memory_consolidation_runs` row, and the row carries:
  - `trigger`, which is `nightly`, `manual` or `full`;
  - counts and spend;
  - `report`: a JSON object holding `operations` (`index`, `operation`,
    `reason`, `before` and `after` snapshots of every record the operation
    touched, and `undoneAt`), `kept`, `refused` by reason, `invalid`,
    `deferred` and `batches`.
- A dream with no records settles `completed` with `no_candidates`.
- If the first batch fails, the dream is `failed`. If a later batch fails, it
  stays `completed`, and the failure is named in the detail.
- The lease is renewed between batches. A `MemoryChanged` event is published
  after any batch that applied something.

### Undo

- `memory/undo_dream_operation {runId, opIndex}` undoes one operation. It is
  refused, changing nothing, in any of these cases:
  - the operation is already undone;
  - a record it touched has changed since; its status or `updated_at`
    differs from the `after` snapshot, except where the expiry sweep closed an
    expiry the dream set;
  - the restored records would take the scope past its budget.
- What undo restores for each operation:

  | Operation | Undo |
  |---|---|
  | `merge` / `correct` | The successor is closed (`superseded`). Each source comes back as a new record that keeps its body, kind, provenance, confidence, rationale, source session, conflict group, expiry, `created_at` and `last_used_at`. Active sources come back active. Proposal sources come back waiting, with their own `supersedes`. |
  | `expire` | The old expiry is put back. If the sweep already closed the record, it is restored. |
  | `group` | Closed members are restored, and the survivor's old group is put back. |
  | `retire` | The record is restored. |
  | `approve` | The approved record is closed, and the proposal comes back waiting. A target it swapped out comes back active. |
  | `reject` | The proposal goes back to waiting, in place. |

- Every record an undo restores is held.
- `memory/undo_dream {runId}` undoes every operation that is not already
  undone, in reverse order, in one transaction. Operations that changed since
  are skipped and listed. The budget is checked on the total.
- As-of reads:
  - before the dream, the read answers the pre-dream active set;
  - between the dream and the undo, it answers the dream's set;
  - after undo all, the active set has the same (body, kind, provenance)
    multiset as before the dream.
- An undo publishes `MemoryChanged`.

### Wire (protocol 1.63, additive)

- `MemoryConsolidationSettings.dreamHour` (default 3), and
  `UpdateConsolidationSettingsParams.dreamHour` (optional, 0–23).
- `MemoryConsolidationRun.id` and `.trigger` (optional).
- `MemoryActivityEntry` gains three fields:
  - `runId` (optional);
  - `spendMicrousd` (default 0);
  - `dream` (optional `MemoryDreamTally`: merged, corrected, expired, grouped,
    retired, approved, rejected, kept, undone).
- New methods:

  | Method | Params | Returns |
  |---|---|---|
  | `memory/start_dream` | — | `MemoryDreamStarted {runId, alreadyOpen}` |
  | `memory/get_dream_report` | `{runId}` | `MemoryDreamReport` |
  | `memory/undo_dream_operation` | `{runId, opIndex}` | `MemoryDreamUndoResult {report, skipped}` |
  | `memory/undo_dream` | `{runId}` | `MemoryDreamUndoResult {report, skipped}` |

  - `MemoryDreamReport` holds `runId`, `trigger`, `status`, `at`,
    `spendMicrousd`, `operations`, `kept`, `refused` and `deferred`.
  - Each entry in `operations` has `index`, `operation`, `reason`, and
    `before` and `after` as `MemoryDreamRecord {id, body, kind, status, pin,
    expiresAt}`, plus `undoneAt`.
- Tauri commands mirror the four methods, and the `api.ts` mock serves them.

### UI (Memory screen, Activity tab)

- A consolidation entry reads like "Dreamed at 03:02: merged 6, approved 4,
  rejected 12, retired 3 · $0.04":
  - zero counts are left out;
  - "Dreamed at 03:02: nothing to change" when nothing applied;
  - failures keep the existing status line.
- Clicking a dream entry expands its operations. Each one shows:
  - a label ("Merged 4", "Approved", "Rejected", …);
  - the before bodies, the after body, and the reason;
  - an **Undo** button, which reads "Undone" once done.

  The expanded entry also has **Undo all**.
- The Recent runs header has a "Dream now" button and a "Nightly cleanup"
  On/Off choice. Busy states use `pendingClass`.

## Unit Tests

### Cadence (`memory_consolidation::tests`)
- `a_dream_is_due_at_the_dream_hour_and_not_a_minute_before`: 02:59 is not
  due and 03:00 is, local offset applied.
- `a_missed_night_dreams_on_the_first_tick_after_launch`.
- `no_automatic_dream_within_twenty_hours_of_the_last`: a manual dream at
  10:00 means 03:00 is not due and 06:00 is.
- `nothing_changed_since_the_last_dream_skips_it`.
- `a_full_scope_obeys_the_floor_and_the_change_check`.
- `dream_now_queues_once_and_bypasses_the_cadence`.
- `a_finished_turn_no_longer_queues_consolidation`.
- `nightly_cleanup_is_on_by_default_and_off_cancels_automatic_runs_only`.

### Input (`memory_consolidation::tests`)
- `dream_input_holds_active_and_waiting_records_with_aliases`.
- `similar_records_share_a_batch`.
- `batches_are_bounded_and_the_rest_is_deferred`.

### Guards (`memory_consolidation::tests`)
- `pins_and_held_records_accept_only_keep`.
- `a_raw_id_or_another_batchs_alias_is_refused`.
- `an_ungrounded_merge_or_correction_is_refused`.
- `approve_and_free_reject_exist_only_in_automatic_mode`.
- `a_review_first_reject_needs_a_similar_duplicate`.
- `approve_refuses_transient_duplicate_and_over_budget_proposals`.
- `merging_proposals_shrinks_the_queue`.
- `a_record_takes_one_operation_per_dream`.
- `the_spend_cap_stops_further_batches_and_the_operation_cap_refuses`.
- `retire_is_a_restorable_retirement`.

### Report and undo (`memory_consolidation::tests` or `memory_dream::tests`)
- `a_dream_writes_one_run_with_a_report_of_every_change`.
- `undo_of_a_merge_restores_the_originals_exactly`.
- `undo_of_each_operation_kind_restores_its_before_state`: correct, expire
  (live and swept), group, retire, approve (plain and swap), reject.
- `undo_all_restores_the_pre_dream_state_and_as_of_reads_hold`.
- `undo_refuses_a_changed_record_and_changes_nothing`.
- `undo_respects_the_budget`.
- `a_restored_record_is_kept_by_the_next_dream`.

### Protocol (`bridge-protocol`)
- camelCase shapes and defaults for the new and extended types, the four
  methods in the method table, and the 1.62 → 1.63 pairing test.

### Frontend (`MemoryDialog.test.tsx`)
- A dream entry reads "Dreamed at …: merged 2, approved 1 · $0.04".
- Expanding shows the operations. Undo calls `undoDreamOperation` once and
  shows "Undone". Undo all calls `undoDream`.
- Dream now calls `startDream` once.

## Integration / Functional Tests
- `memory_evals::dream_cases_hold_offline` runs in the normal suite. For every
  labeled dream case, a canned ideal answer passes the gate and meets the
  case's expectation. Canned adversarial answers are refused: a pin edit, a
  foreign id, an ungrounded merge, and approving junk.
- `memory_properties` gains a property: for any answer text, a dream never
  changes a pin, never exceeds the budget, and undo all restores the active
  multiset.
- Golden off-trace: unchanged unless the trace covers consolidation; any
  change is explained in the PR.
- Tauri `every_command_signature_matches_its_contracted_params` passes with
  the four commands.

## Smoke Tests
- `cargo test -p bridge-core --lib memory_`, `-p bridge-protocol`,
  `-p bridged`, and `-p bridge-deck --lib`.
- `bunx vitest run src/components/MemoryDialog.test.tsx src/api.test.ts`.
- `bunx tsc -b`, `bun run lint` and `bun run check:visual`.

## E2E Tests (model evals; results posted on the PR)
- **Labeled cases** live in `testing/evals/memory-dream/cases.json`:

  | Case | Expectation |
  |---|---|
  | Four "independent researcher" variants | One merge |
  | Two em-dash duplicates | Merge |
  | Two duplicate waiting proposals | Merge, or reject one as a duplicate |
  | A contradicted preference (old active, newer proposal; Automatic) | The new claim is active and the old one is not |
  | A dated one-off ("demo on Friday", already past) | Expire or retire |
  | A pin with a paraphrased waiting proposal | The pin is untouched and the proposal rejected |
  | A junk status-update proposal | Reject; approving it is a wrong approval |
  | A good durable proposal (Automatic) | Approve |
  | The junk and duplicate cases in Review-first mode | No approval |
  | A tidy scope | Only `keep` (churn 0) |

- **`memory_evals::live_dream`** is ignored by default. It needs
  `BRIDGE_MEMORY_EVAL=1` and a real login, and runs through
  `BRIDGE_MEMORY_EVAL_MODEL`.
  - It runs once per model: `claude-haiku-5-5`, `claude-sonnet-5-5` and
    `claude-opus-5-5`. Opus is what the owner's memory model resolves to. Each
    model gets 3 repeats.
  - Gates for each model: pins changed = 0, invented facts landed = 0, merge
    recall ≥ 80%, wrong approvals = 0.
  - It also reports:
    - ungrounded attempts the gate refused;
    - contradiction resolved, expiry hit and approval recall;
    - tidy-scope churn, spend and wall time.
  - Each model runs in a dedicated `CARGO_TARGET_DIR`, sandboxed per the
    third-party-tool rule.
- **Real-data dry run.** A read-only copy of the local database goes into a
  temp dir, and one full dream runs there on each model.
  - It reports aggregates only: operations by kind, refusals by reason,
    batches, deferred records, spend and wall time.
  - No memory text leaves the machine. Per-operation examples are reviewed
    locally and summarized to the owner in chat, not on the PR.
- The PR carries a comment with a per-model results table and the chosen
  default.

## Manual / cURL Tests
- N/A for cURL: no HTTP surface.
- In mock mode (`bun run dev`), check that the dream entry renders, expands
  and undoes, and that Dream now and Nightly cleanup render. Screenshots go in
  the PR.
