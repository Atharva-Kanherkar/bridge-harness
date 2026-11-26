# feat/memory-idle-extraction — Test Contract

Issue #1148, part of epic #1144. Locked before implementation. Plan note:
"Plan: extract once per idle chat and reconcile (#1148)".

## Functional Behavior

### Scheduling

- A completed turn in a visible chat (direct or orchestrator) with extraction
  on, a resolved memory model and room in the review queue keeps **one queued
  run per chat** and sets its `due_at` to `now + 1,200 s` (20 minutes). Every later turn moves
  `due_at` out again; it never adds a second queued run.
- A queued run is claimed only once `due_at <= now`, and never while the same
  chat has a running run with a live lease. A running run does not stop a new
  turn from queueing the next one.
- Archiving a chat makes its queued run due at once.
- Existing queued runs from before the upgrade are due at their `created_at`,
  so nothing waits longer than it would have.
- 30 turns one minute apart followed by quiet produce **one** run, not 30.

### Watermarked digest

- Each completed run records `through_entry_id`, the last entry its digest
  covered. The next digest reads only entries after the last completed run's
  watermark for that chat, oldest first.
- A chat with no completed run yet reads at most its last 60 entries.
- Past completed runs are backfilled with the newest conversational entry
  created at or before the run was queued, so the upgrade does not re-read old
  chats from the start.
- A digest stops at 24,000 characters. Assistant messages are clipped to 600
  characters, since they are context, never evidence. A chunk that leaves
  entries behind queues a follow-up run that is due at once.
- With no new user message and no new in-scope app activity after the
  watermark, the run is settled `cancelled` with detail `nothing_new` and no
  model call.

### Similar memories (reconcile context)

- Before the call, up to 8 `active` or `proposed` records in the scope are
  chosen by term overlap with the **user's** new messages and in-scope app
  activity (not assistant text), using `memory_similarity::terms`. A record
  needs at least 2 shared terms, or a single term covering at least half the
  record's terms. Records are ranked by coverage, then shared count, then pins
  first.
- They appear in the digest as `[m1] pin · preference: <body>` lines (bodies
  clipped to 300 characters), replacing the old "Already pinned" list. The
  alias map from `m1..m8` to record ids stays server-side.

### Operations and gate

- The fenced `bridge-memory-proposals` array holds operations:
  - `{"op":"add","body","kind","confidenceBps","rationale"}`.
  - `{"op":"update","target":"m2","body","kind"?,"confidenceBps","rationale"}`.
  - `{"op":"noop","target":"m1"}`.

  An element without `op` is an add. An unknown `op` or any extra field is
  invalid.
- `update` and `noop` targets must be aliases shown in this run. Anything else
  is invalid and counted, and nothing is written for it.
- A pin (`user_explicit`) accepts only `noop`. An `update` of a pin is refused.
- An `update` needs a rationale citing a stable user message from the chat. It
  lands as `proposed` with `supersedes = target` and never auto-applies.
- An `update` whose body is the same claim as its target counts as already
  known.
- An `update` of a waiting proposal supersedes that proposal at once, so the
  queue does not grow.
- Approving a proposal whose `supersedes` target is still active activates it
  and supersedes the target in one transaction. It does not count against the
  active budget, single or in bulk.
- `noop` writes nothing and adds 1 to the run's `known_count`.
- `add` keeps every existing rule: the duplicate and near-duplicate refusals,
  the batch conflict guard, the queue cap and the automatic-approval criteria.

### Wire and UI

- Protocol 1.61 adds `knownCount` (default 0) on `MemoryExtractionRun` and
  `known` (default 0) on `MemoryActivityEntry`. An extraction activity entry
  reports its run's `known_count`.
- Memory screen, Review tab:
  - The mode heading reads "When a chat goes quiet".
  - The last-run line adds "· N already known" when N > 0.
- A waiting proposal that supersedes a memory shows "Replaces: <old body>".
- Activity: an extraction entry adds ", N already known". A `nothing_new`
  cancellation reads "Nothing new to read".

## Unit Tests

### Scheduling (`memory_extraction::tests`)
- `a_turn_re_arms_the_one_queued_run`: three turns, one row, and `due_at` moves.
- `a_queued_run_is_not_due_before_the_quiet_period`: not claimed before 1,200 s,
  claimed after.
- `archiving_makes_the_queued_run_due_now`.
- `a_running_run_blocks_a_second_claim_for_the_same_chat`.
- `thirty_turns_a_minute_apart_make_one_run`.

### Digest (`memory_extraction::tests`)
- `the_digest_reads_only_entries_after_the_watermark`.
- `a_first_run_reads_at_most_the_last_sixty_entries`.
- `assistant_text_is_clipped_and_user_text_is_not`.
- `a_long_backlog_is_read_in_chunks_with_a_follow_up_run`.
- `no_new_user_message_cancels_without_a_model_call`.
- `migration_backfills_watermarks_from_past_runs`.

### Similar memories (`memory_extraction::tests`)
- `similar_memories_come_from_user_words_not_assistant_words`.
- `similar_memories_include_waiting_proposals_and_cap_at_eight`.
- `unrelated_memories_are_not_offered`.

### Gate (`memory_extraction::tests`)
- `noop_writes_nothing_and_counts_known`.
- `update_lands_as_a_proposal_that_supersedes_on_approval`.
- `update_of_a_waiting_proposal_replaces_it_in_the_queue`.
- `targets_must_come_from_the_prompt`: an unknown alias and a raw id are both
  invalid.
- `a_pin_accepts_only_noop`.
- `update_without_a_cited_user_message_is_refused`.
- `update_never_auto_applies`.
- `an_element_without_op_is_an_add`.

### Ledger (`memory_ledger::tests`)
- `approving_an_update_supersedes_its_active_target_in_one_step`.
- `approving_an_update_at_full_budget_is_allowed`, single and in bulk.
- `rejecting_an_update_leaves_the_target_active`.

### Protocol (`bridge-protocol`)
- camelCase `knownCount` and `known` with default 0, and the 1.60 → 1.61
  pairing test.

### Frontend (`MemoryDialog.test.tsx`)
- The last-run line shows "3 already known".
- A proposal with `supersedes` shows "Replaces: <old body>".
- The heading reads "When a chat goes quiet".
- A `nothing_new` activity entry reads "Nothing new to read".

## Integration / Functional Tests
- `memory_evals::reconcile_cases_surface_their_target` (offline, runs in the
  normal suite). For every labeled reconcile case, the expected target is among
  the similar memories in the digest.
- `memory_properties`: existing properties still hold with operations in the
  model text (gate writes only proposals; budget never exceeded).
- Golden off-trace is unchanged except for scheduling fields if the trace
  covers extraction runs; any change is explained in the PR.

## Smoke Tests
- `cargo test -p bridge-core --lib memory_`, `-p bridge-protocol`,
  `-p bridge-deck --lib`, `bunx vitest run src/components/MemoryDialog.test.tsx
  src/api.test.ts`, `bunx tsc -b`, `bun run lint`, visual check.

## E2E Tests
- `memory_evals::live_reconcile` (ignored; needs `BRIDGE_MEMORY_EVAL=1` and a
  real harness login). Labeled cases:
  - a restated known fact expects `noop`;
  - a changed preference expects `update` on the right id;
  - a new durable fact expects `add`;
  - an assistant-only claim expects nothing;
  - a status update expects nothing;
  - a restated pin expects `noop`;
  - a changed pin expects that the pin is not edited.

  Gates: duplicate proposal rate ≤ 10%, wrong-target updates = 0, pins edited
  = 0. It prints the spend. Run once on the memory model the PR names, with a
  dedicated `CARGO_TARGET_DIR`.
- **Replay estimate:** a read-only script over the local database's last 7
  days counts the runs the old rule made against the runs the new rule would
  make (20-minute quiet periods per chat). It scales cost by the observed cost
  per run. Target: ≥ 5x fewer runs.

## Manual / cURL Tests
- N/A for cURL: no HTTP surface.
- Manual, in mock mode (`bun run dev`): the Review tab heading, the
  already-known count, and the "Replaces" line render. Screenshots go in the PR.

## Amendment: quiet period (decided after the replay)

The issue specified 10 minutes. Replaying the last 7 days of the local
database, measuring quiet time from the end of each turn, gave:

| Quiet period | Runs | Fewer than the old rule |
|---|---|---|
| Old rule | 737 | — |
| 10 min | 188 | 3.9x |
| 20 min | 145 | 5.1x |
| 30 min | 125 | 5.9x |

At 10 minutes the run count misses the 5x target. The owner chose 20 minutes
on 2026-11-26. Archiving a chat still makes its run due at once.
