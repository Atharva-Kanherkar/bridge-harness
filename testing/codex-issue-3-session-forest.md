# codex/issue-3-session-forest — Test Contract

## Functional Behavior

- `src-tauri/src/session_forest.rs` is the typed, append-only API over `session_entries` and `session_heads`.
- Every durable entry kind listed in issue #1 has a typed enum value and kind-specific payload validation; compatibility-only legacy kinds may be read but cannot be appended through the typed API.
- Appending uses the active head by default; appending from an earlier entry creates a branch without changing or deleting prior entries, and sequence remains global per-session insertion order.
- Navigation changes only `session_heads.active_entry_id`; no entry row is updated or deleted and no filesystem operation occurs.
- Active-branch traversal is iterative, deterministic, root-to-leaf, and safe for at least 10,000 entries without stack growth.
- Queries expose children, branch leaves, branch summaries, and active-leaf navigation/forking.
- Missing parents, cross-session parents, cycles, invalid heads, unknown append kinds, and invalid payloads return typed errors rather than panicking.
- Corrupt sessions are reported as quarantined by a forest integrity scan while valid sibling sessions remain queryable.

## Unit Tests

- `all_entry_kinds_validate_their_payload_contract` — every #1 entry kind accepts a valid payload and rejects a missing required field.
- `append_rewind_append_preserves_immutable_history` — a property-style operation sequence never changes or removes prior entries.
- `active_branch_traversal_is_deterministic` — repeated traversal from the same head returns identical root-to-leaf IDs.
- `forks_share_a_prefix_and_diverge_after_the_fork_point` — two leaves share the expected prefix and retain both suffixes.
- `branch_summary_is_an_immutable_entry` — summaries are typed entries on the selected branch.
- `traverses_ten_thousand_entries_without_recursion` — the complete chain is returned without stack growth.
- `detects_and_quarantines_orphans_cross_session_parents_cycles_and_bad_heads` — each corruption yields a typed finding and healthy sibling traversal still succeeds.
- `public_navigation_never_mutates_entries` — row payloads/counts remain byte-for-byte stable across head moves.

## Integration / Functional Tests

- The module uses the schema and store types landed in #2 and is registered in the Tauri crate.
- A migrated compatibility forest remains readable; known durable kinds validate while legacy-only kinds remain inspectable but cannot be newly appended.
- `cargo test --manifest-path src-tauri/Cargo.toml session_forest` passes.
- `cargo test --manifest-path src-tauri/Cargo.toml` passes.

## Smoke Tests

- `bun run check` passes.
- `bun run test` passes.
- `bun run build` passes.

## E2E Tests

- N/A for UI in this Phase 1 PR; branch navigation UI is issue #12. The in-memory SQLite integration tests execute the full append → rewind → fork → navigate → traverse journey.

## Manual / cURL Tests

- N/A for cURL — no HTTP API changes.
- Inspect `session_entries` before and after navigation and confirm only `session_heads` changes.
- Confirm `rg 'UPDATE session_entries|DELETE FROM session_entries' src-tauri/src/session_forest.rs` returns no production mutation statements.
