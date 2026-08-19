# feat/work-reconciliation — Test Contract

Issue #208, slice 5 of 7 under epic #203. Locked before implementation.

Slices 1–4 are merged. Slice 4 produces a validated brief and an evidence ledger and
deliberately writes no `work_tasks`. This slice is the writer, plus the actions a human
takes on what it wrote.

## The shape of the problem

A briefing runs repeatedly against sources that change. So the hard part is not turning
a brief into rows — it is deciding, for each row, whether it is the *same task as last
time*. Three things follow.

**Identity has to survive the model.** A task is the same task because it points at the
same resource in the same account, not because the model phrased it the same way. So
identity is a fingerprint over Bridge-derived facts, and it has to distinguish two
accounts that number their issues identically.

**Absence has to mean something.** A task that stops appearing might be finished, or its
source might have been down. Only a *successful* read of that task's own source is
evidence it is gone, which is why the miss count is stored rather than derived.

**A human's decision outranks the model's.** Done, dismissed, snoozed and pinned are the
user talking. A reconciliation that reset them because the brief changed would make those
buttons decorative.

## Functional Behavior

### Fingerprints

- Versioned and separator-safe: `v1` plus a length-prefixed or escaped encoding of
  connector instance and canonical resource id, so no pair of inputs can be arranged to
  produce another pair's fingerprint.
- Two accounts, repositories, teams, or channels holding the same native id produce
  different fingerprints. Asserted with real collision attempts, not just two happy cases.
- A task Bridge cannot identify has **no** fingerprint and is ephemeral.
- The version is part of the value, so a future encoding can coexist rather than silently
  matching the wrong rows.

### Reconciliation is one transaction

- Run status, source coverage, evidence, task upserts, state preservation, usage, and the
  board all commit together. A reader sees the old board or the whole new one, never half.
- Replaying the same run changes nothing — asserted by running it twice and comparing the
  full table.
- A run that fails commits its coverage and evidence and leaves the previous board intact.

### Aging is source-scoped

- A task's miss count increments only when *its own* source was read successfully in that
  run and the task was not in the brief.
- A connector failure, an auth requirement, a provider failure, a budget breach, or an
  ineligible source does **not** age tasks from that source. Each is its own test.
- Two successful misses make a task `stale`. One does not.
- A task that reappears resets its miss count.

### States, and pinning across them

- `active`, `snoozed`, `done`, `dismissed`, `stale`; `pinned` is orthogonal to all five.
- `snoozed` tasks keep reconciling, so a snooze that expires reveals current information
  rather than a snapshot from when it was set.
- `done` reopens **only** on evidence newer than the resolution. Same-or-older evidence
  leaves it done.
- `dismissed` stays suppressed until an explicit restore — a later brief does not undo it.
- `pinned` tasks stay visible when they go stale, rather than disappearing.
- Ephemeral tasks survive only until the next **successfully committed** briefing. A
  failed run does not clear them.
- Every illegal transition is refused, and the legal set is enumerated in one place.

### Actions write nothing outward

- `done`, `snooze`, `dismiss`, `pin`, `restore`, and `prepareSession` perform **no**
  connector write and no provider turn. Asserted by a spy over the whole surface.
- `prepareSession` creates or opens a normal session with an editable draft built from
  untrusted task text, and dispatches nothing until the user presses Send.

### Evidence opening is checked at the point of opening

- Accepts only a Bridge-derived HTTPS target on a provider host, or a typed local target.
- Rejects `http:`, `file:`, custom schemes, malformed URLs, unexpected hosts, and
  anything the stored row could have been tampered into holding.
- Rechecked on open rather than trusted because it was checked on write: the row is the
  attacker-reachable surface once a database exists.

## Unit Tests

`work_fingerprint`:

- `two_accounts_with_the_same_native_id_do_not_collide`
- `separator_stuffing_cannot_forge_another_fingerprint` — inputs arranged so a naive
  `a:b` concatenation would collide.
- `the_version_is_part_of_the_value`
- `an_unidentifiable_task_has_no_fingerprint`
- `the_same_inputs_always_produce_the_same_fingerprint`

`work_tasks` state machine:

- `only_a_successful_read_of_a_tasks_own_source_ages_it`
- `a_connector_failure_does_not_age_its_tasks`
- `an_auth_required_source_does_not_age_its_tasks`
- `an_ineligible_source_does_not_age_its_tasks`
- `two_successful_misses_make_a_task_stale_and_one_does_not`
- `a_reappearing_task_resets_its_miss_count`
- `a_snoozed_task_keeps_reconciling`
- `a_done_task_reopens_only_on_newer_evidence`
- `a_dismissed_task_stays_suppressed_until_restore`
- `a_pinned_task_stays_visible_when_it_goes_stale`
- `pinning_is_orthogonal_to_every_state`
- `an_ephemeral_task_survives_only_until_the_next_committed_run`
- `a_failed_run_does_not_clear_ephemeral_tasks`
- `every_illegal_transition_is_refused`

Reconciliation:

- `a_reader_sees_the_old_board_or_the_whole_new_one`
- `replaying_the_same_run_changes_nothing`
- `a_failed_run_keeps_the_previous_board`
- `a_partial_run_ages_only_the_sources_it_read`
- `human_state_survives_a_reconciliation`

Actions:

- `no_action_performs_a_connector_write_or_a_provider_turn`
- `prepare_session_creates_a_draft_and_dispatches_nothing`
- `a_draft_carries_untrusted_text_as_text`

Evidence opening:

- `only_https_on_a_provider_host_opens`
- `a_tampered_stored_target_is_refused_on_open`
- `http_file_and_custom_schemes_are_refused`
- `a_typed_local_target_opens`

## Integration / Functional Tests

- A full run commits a board; a second run with one task absent and its source read
  successfully ages that task; a third does the same and it goes stale.
- The same three runs where the source *failed* leave the task untouched throughout.
- A run that fails to parse leaves the committed board byte-identical.

## Smoke Tests

- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` green.
- `npx vitest run` green.
- `bun run build` green.
- `npm test --prefix sidecar/claude-agent` green.
- Protocol artifacts regenerate with no drift, or the regenerated files are committed in
  the same step.

## E2E Tests

N/A — no browser-driven harness exists in this repo. Component tests drive real DOM via
`react-dom/client`, which is what every other screen here uses.

## Manual / cURL Tests

No HTTP surface.

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core work_fingerprint work_reconcile
```
