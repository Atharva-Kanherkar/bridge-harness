# fix/import-purge-fast: Test Contract

The cleanup of imported runs (#1103) stalled Bridge: on this Mac's database (1.5 million history entries) it held Bridge's shared database for about 20 minutes per batch, and would have taken about 15 hours in all. #1105 took it out of startup; it still ran after every import.

Measured on a copy-on-write clone of that database:
- With foreign key checks on, every deleted entry made SQLite read the whole history: `session_entries.parent_entry_id` (and `session_heads.active_entry_id`, `latest_checkpoint_entry_id`) have no index. That was 0.6 s per entry.
- The search index's delete trigger removes each entry's search row by `entry_id`, which the index cannot look up without reading all of it. That was 0.06 s per entry, and 1.7 s per batch once done per batch.

## Functional Behavior

- A purge batch removes a run's own rows with foreign key checks off: its session and subagents, history, head, native ids and search rows.
  - This applies only when nothing else refers to the batch; otherwise that batch runs with the checks on, as before.
  - The checks are always back on after the batch.
- The batch's search rows and native ids are found once before the first batch. The search index is read in steps of 20,000 rows, each step under the lock. Batches then delete by row number and key.
- The guard on imported history and the search index's per-entry delete trigger are lifted inside each batch's transaction and restored, exactly as they were, before it commits.
- Folder checks run without Bridge's database held. Batches are 200 runs, with a 50 ms pause between them.
- `purge_background` reports how many runs it removed, the number of batches and the longest any step held the database.
- Startup still does not tidy (#1105). Cleanup runs after an import.

## Unit Tests

- A removed run leaves no search rows, digest or lifted trigger, and foreign key checks come back on. A kept chat keeps its search rows.
- A run something else points at (a title job, cascading) is removed with the checks on, and the cascade is carried out.
- On a 400,000-entry history, 300 runs are removed holding the database under 1.5 s at most (8.9 ms measured); the old checked path holds it for 6.4 s and fails the test.
- The existing purge, placement and job tests.

## Real data (by hand, on a clone)

- `BRIDGE_PURGE_COPY=<clone>/bridge.db cargo test -p bridge-core --lib -- --ignored measure_the_purge_on_a_copy --nocapture`: 27,021 runs removed in 20 s, longest hold 491 ms. Afterwards foreign keys are on, both triggers are back, and there are no orphan entries, heads, search rows, native ids or digests. `quick_check` is ok.
