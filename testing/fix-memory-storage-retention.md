# fix/memory-storage-retention — Test Contract

## Functional Behavior

- Recompiling a memory packet for the same chat replaces that chat's prior audit instead of appending another full copy of the selected pin bodies.
- Different chats retain independent latest packet audits, so the disclosure for one chat cannot overwrite another.
- Upgrading an existing database collapses historical duplicate packet audits before enforcing one audit per recipient chat.
- A successful database open retains only the newest Bridge migration backup and removes older `bridge.db.backup-*` copies.
- Backup cleanup never removes the live database, the newest rollback backup, or unrelated files in the data directory.
- Memory packet selection, citation bodies, exclusions, and disabled-injection behavior remain unchanged.

## Unit Tests

- `recompiling_a_session_replaces_its_audit_instead_of_growing_storage` — two packet builds for one session leave one row containing the newest frozen payload.
- `packet_audits_remain_independent_between_sessions` — compiling two sessions leaves one readable audit for each.
- `migration_45_collapses_duplicate_memory_audits` — an upgraded schema keeps the newest row per recipient and rejects future duplicates at the database constraint.
- `migration_backup_retention_keeps_only_the_newest_bridge_backup` — old timestamped backups are removed and the newest survives.
- `migration_backup_retention_preserves_unrelated_files` — narrowly named cleanup does not touch foreign files or the primary database.

## Integration / Functional Tests

- `cargo test -p bridge-core memory_packet` passes.
- `cargo test -p bridge-core migration_backup_retention` passes.
- `cargo test -p bridge-core migration_45` passes.
- `bun run build` passes.
- `bun run test` passes, including the full Rust workspace.

## Smoke Tests

- Open a copy of a current-schema database with several timestamped migration backups; verify only the newest backup remains after a successful open.
- Build a memory packet repeatedly for one recipient session; verify `memory_retrieval_audits` stays at one row for that session.

## E2E Tests

N/A — the bug is in native persistence and retention; deterministic store-level tests cover the user-visible invariant without launching a provider or mutating the user's live data.

## Manual / cURL Tests

N/A — Bridge is a Tauri desktop app and these persistence paths are not HTTP endpoints. The user's live database is inspected read-only only; no cleanup is performed on user data during development.
