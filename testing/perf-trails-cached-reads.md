# perf-trails-cached-reads — Test Contract

Stacked on `feat/trails-issues-live-eval`. Plan note:
`bridge://p/01a1db22-41d0-7590-9320-f584037454e2`.

Every trail read gets faster and repeated reads are served from a cache that
can never be stale. Nothing a read returns changes.

## Functional Behavior

### Results do not change

- `list_trails`, `read_trail`, `preview_trail_brief`, `carry_trail_brief`
  and the sweep return exactly what they return today for the same store.
  Every existing `trails::` test passes unchanged, except where a test named
  `all_notes` on a fake (the seam changes; the facts it returns do not).

### Targeted notes

- `NotesService::pages(ids)`: the facts and body of exactly those pages,
  archived included, deleted excluded, in any space, one query.
- `NotesService::linked_to(sessions)`: the facts of every live page in the
  service's space (archived included, the system database excluded) whose
  `session_ids` contains any of `sessions`, one query.
- `NoteSnapshot::for_trails(db, notes)` holds the member notes of every trail
  (with bodies) and the notes linked to every chat on an open trail. Notes that
  are neither are never read.
- `sweep_and_publish` sweeps again while a sweep added something, with a fresh
  snapshot each time, so a chat that joins mid-sweep brings its linked notes in
  the same call. It stops after 8 rounds.

### The stamp

- `Stamp` = the core connection's `total_changes` and `PRAGMA data_version`,
  and the notes connection's. Reading it is O(1) and takes each lock briefly.
- Any committed write changes the stamp: through the core connection, through
  the notes service, or through another connection to either file.

### The cache (`trails::cache`)

- Summaries (list) and details (read) are kept with the stamp they were
  derived under, taken while holding the lock they are derived under. A read
  whose stamp matches is served from the cache; any other read derives and
  replaces the entry.
- The read-time sweep in `list_trails` and `read_trail` is skipped when the
  stamp has not moved since the last completed sweep. The maintenance loop
  always sweeps.
- `read_trail {refresh: true}` never serves a cached detail.
- The cache holds at most the trails that exist; an archived or missing trail
  is not served from it.

### Brief

- Chat history items are cached by `(session id, active head entry id,
  window)`. Appending to a chat moves its head, so the next preview projects
  again. At most 256 projections are kept; the oldest go first.
- Note items read the body through `NotesService::pages`.
- Preview and carry produce the same items, text and token counts as today.

### Index

- Migration 79: `CREATE INDEX idx_session_pull_requests_url ON
  session_pull_requests(url)`. `EXPLAIN QUERY PLAN` for `pull_facts`' lookup
  uses it.

### Frontend

- `bridgeApi.listTrails(includeArchived)` calls with the same argument made
  in the same task (the sidebar badge and the Trails screen both answer one
  `trails/changed`) share one request, sent after the last of them. A call
  made after that request was sent makes a new one, so no caller ever gets an
  answer older than its own call. (Amended before code: sharing a request
  that is already in flight could hand a caller data from before the change
  it is reacting to.)

## Unit Tests

- `notes::service::tests::pages_and_linked_to_read_only_what_they_name`
- `trails::timeline::tests::snapshot_for_trails_reads_members_and_linked_notes_only`
- `trails::live::tests::a_chat_that_joins_mid_sweep_brings_its_linked_note`
- `trails::cache::tests::a_write_through_the_core_invalidates`
- `trails::cache::tests::a_write_through_the_notes_store_invalidates`
- `trails::cache::tests::a_write_from_another_connection_invalidates`
- `trails::cache::tests::an_unchanged_stamp_serves_the_same_detail_without_deriving`
- `trails::cache::tests::the_read_sweep_runs_again_after_any_write`
- `trails::brief::tests::chat_history_is_reprojected_after_a_new_entry`
- `store::tests::migration_79_indexes_pull_request_urls`
- Vitest: `api.trails.test.ts` same-task `listTrails` calls share one request; a call after it was sent makes another.

## Integration / Functional Tests

- All existing `trails::`, `github_poll::`, `session_prs::` tests.
- `trails::bench::reads` (release, ignored) reports warm and cold numbers.

## Performance (interleaved A/B, frozen release binaries, same store)

Baseline medians (old binary): list 17.9 ms, read 14.5 ms, preview 8.4 ms,
sweep 8.2 ms. New binary, median of the interleaved runs:

- Warm (no write since the last read): list, read, preview each under 1 ms.
- Cold (one chat entry appended before each call): list and read at most half
  of baseline; sweep at most half of baseline.

## Smoke Tests

- `bun run build`, `bun run test` green.

## E2E Tests

- N/A: no UI change beyond request sharing, covered by vitest.

## Manual / cURL Tests

- Scratch `bridged`: create a trail, list twice (second served warm), append
  to a member chat, list again (fresh). Notes edit through the notes service:
  list reflects the new title.
