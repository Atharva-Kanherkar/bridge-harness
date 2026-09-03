# fix/memory-storage-retention — Test Contract

## Functional Behavior

- Recompiling a memory packet for the same chat keeps the append-only delivery audit but compacts prior rows to selected ids, so full pin bodies are not duplicated indefinitely.
- Different chats retain independent latest packet audits, so the disclosure for one chat cannot overwrite another.
- Upgrading an existing database compacts historical full-body packet copies while preserving every delivery row and the latest frozen disclosure.
- Old Bridge writers remain compatible with the upgraded schema; their plain inserts trigger the same bounded-body compaction.
- Out-of-order audit inserts are compacted immediately and cannot displace the latest packet's full frozen disclosure.
- Deleted sessions purge their packet audits so full memory bodies do not become orphaned.
- Upgrade also purges packet audits already orphaned by sessions deleted before migration.
- Database open prunes older valid backups before migration, then protects the rollback copy created by the current migration regardless of clock-skewed filenames.
- Migration backups are consistent SQLite snapshots staged under a temporary name and atomically published only after completion.
- Backup cleanup never removes the live database, the newest rollback backup, or unrelated files in the data directory.
- When multiple rollback candidates exist, cleanup keeps a structurally readable SQLite backup instead of trusting a newer header-only or truncated file.
- Crash-leftover backup staging files with exact Bridge-generated names are reclaimed on the next open.
- Memory packet selection, citation bodies, exclusions, and disabled-injection behavior remain unchanged.

## Unit Tests

- `recompiling_a_session_compacts_the_prior_payload_instead_of_duplicating_it` — two packet builds retain both audit rows, compact the prior row to ids, and keep the newest frozen payload.
- `recompiling_to_an_empty_packet_still_compacts_the_prior_payload` — a later empty selection does not leave the previous full-body copy behind.
- `packet_audits_remain_independent_between_sessions` — compiling two sessions leaves one readable audit for each.
- `migration_45_compacts_historical_bodies_and_keeps_old_writers_compatible` — equal-timestamp and backdated history preserve the latest full disclosure, every live-session delivery row, pre-upgrade plain-writer compatibility, and purge pre-existing orphan audits.
- `deleting_a_session_purges_its_memory_packet_audits` — session cleanup removes its stored packet bodies.
- `migration_backup_retention_keeps_only_the_newest_bridge_backup` — old timestamped backups are removed and the newest survives.
- `migration_backup_retention_preserves_unrelated_files` — narrowly named cleanup reclaims exact crash-leftover staging files without touching malformed names, foreign files, or the primary database.
- `migration_backup_retention_prefers_the_just_created_rollback` — clock-skewed filenames cannot displace the backup made for the current migration.
- `migration_backup_retention_rejects_a_truncated_newer_backup` — structural validation prevents a corrupt newer candidate from displacing an older recoverable rollback.

## Integration / Functional Tests

- `cargo test -p bridge-core memory_packet` passes.
- `cargo test -p bridge-core migration_backup_retention` passes.
- `cargo test -p bridge-core migration_45` passes.
- `bun run build` passes.
- `bun run test` passes, including the full Rust workspace.

## Smoke Tests

- Open a copy of a current-schema database with several timestamped migration backups; verify only the newest backup remains after a successful open.
- Build a memory packet repeatedly for one recipient session; verify only the newest row retains full selected-item bodies.

## E2E Tests

N/A — the bug is in native persistence and retention; deterministic store-level tests cover the user-visible invariant without launching a provider or mutating the user's live data.

## Manual / cURL Tests

N/A — Bridge is a Tauri desktop app and these persistence paths are not HTTP endpoints. The user's live database is inspected read-only only; no cleanup is performed on user data during development.
