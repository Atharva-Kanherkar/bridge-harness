# codex/issue-2-versioned-migrations — Test Contract

## Functional Behavior

- Opening a fresh database creates the current schema through ordered migrations and records every applied version in `schema_version`.
- Opening a database built by the pre-migration code copies `bridge.db` to a timestamped backup before applying the first pending migration.
- Every migration is transactional, idempotent, and leaves foreign-key enforcement enabled.
- The Phase 1 schema exactly adds `session_entries`, `session_heads`, `task_knowledge`, `worker_leases`, and `usage_ledger` with the constraints and indexes specified in issue #1/#2.
- Existing `agent_events` are backfilled into a per-session linear parent chain in insertion-sequence order; the head points to the final entry and provider metadata remains in the payload.
- New normalized agent events dual-write atomically to legacy `agent_events` and forest `session_entries`; compatibility reads remain on `agent_events` in this PR.
- Store APIs append/query entries, heads, knowledge, leases, and usage rows while retaining one SQLite connection as the writer boundary.
- App startup lifecycle status normalization remains unchanged.

## Unit Tests

- `migrates_current_schema_fixture_idempotently_and_creates_backup` — upgrade twice, record versions once, preserve data, and create exactly one pre-migration backup.
- `fresh_and_upgraded_databases_have_identical_schema` — compare normalized `sqlite_master` definitions and migration versions.
- `backfills_agent_events_as_linear_session_entries` — preserve count/order/payload, parent each entry to its predecessor, and set the correct head.
- `session_entry_sequence_is_unique_and_append_assigns_next_sequence` — duplicate sequences fail and append uses per-session global insertion order.
- `agent_event_dual_write_is_atomic_and_equivalent` — normalized data/provider metadata appears in both representations with matching sequence and kind.
- `foreign_keys_are_enforced` — an entry for a missing session is rejected.
- CRUD/query tests cover task knowledge, worker leases, usage ledger, and session heads.

## Integration / Functional Tests

- `cargo test --manifest-path src-tauri/Cargo.toml --workspace store` passes against temporary on-disk databases.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` passes without changing compatibility replay behavior.
- Failure injected into a transactional migration leaves no partial schema version or partial objects.

## Smoke Tests

- `bun run check` passes.
- `bun run test` passes.
- A fresh temporary `bridge.db` can be opened twice without error.

## E2E Tests

- N/A — this is a persistence foundation PR with intentionally no UI behavior change; migration, replay, and full-suite integration tests cover the externally observable behavior.

## Manual / cURL Tests

- N/A for cURL — no HTTP API changes.
- Inspect a migrated fixture with `sqlite3 bridge.db '.schema'` and confirm schema versions, five new tables, required indexes, backfilled chains, and foreign keys.
- Confirm `rg 'let _ = connection.execute\("ALTER TABLE' src-tauri/bridge-core/src/store.rs` returns no matches.
