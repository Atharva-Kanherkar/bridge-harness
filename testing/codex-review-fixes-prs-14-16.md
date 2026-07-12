# codex/review-fixes-prs-14-16 — Test Contract

## Functional Behavior

- Compatibility dual-write/backfill entries are always readable even when their normalized kind string matches a typed forest kind; kind-specific payload validation applies only to entries created through the typed forest append API.
- Typed forest entries carry an internal schema-source marker, remain validated on traversal/integrity scans, and continue to reject invalid typed payloads.
- Appending from a valid parent uses direct parent/session validation rather than traversing and validating the entire historical branch.
- Archiving a clean, stopped workspace transactionally removes all dependent forest, knowledge, lease, usage, legacy event, session, and workspace rows without foreign-key failures; a failed worktree removal rolls the database deletion back.
- `bridge-worker-result` fences are removed only from rendered conversation text. Raw stored output remains available for repair parsing and inspection.
- Worker result forwarding is factored into a runtime-testable path that records `worker.result.repair_requested` and `worker.result.unstructured` audit events.
- Depth-one delegation attempts and fanout overflow are explicitly rejected with inspectable normalized/audit events; a fully rejected request never displays “Delegating to a worker…”.
- Invalid delegation blocks with no prose render an explicit invalid-request message rather than an empty assistant item.
- Unclosed machine fences do not delete subsequent/raw prose during stripping.
- A busy WAL checkpoint aborts pre-migration backup rather than copying a potentially incomplete database.
- Unsupported request/result schema versions are reported before strict unknown-field decoding.

## Unit Tests

- `compatibility_kind_collision_remains_readable` — dual-write `approval.resolved` with no item ID, traverse the forest, and keep the session healthy.
- `invalid_typed_payload_still_fails_traversal` — typed-source marker preserves strict validation.
- `append_validates_parent_directly` — appending to a valid parent succeeds even with unrelated compatibility history and remains globally sequenced.
- `archive_workspace_records_cleans_every_dependent_table` — all child rows are removed and foreign keys remain enabled.
- `archive_workspace_records_rolls_back_when_worktree_removal_fails` — no database rows are lost on external failure.
- `conversation_hides_worker_result_fence_but_preserves_raw_event` — reducer output omits the block while input event text remains unchanged.
- `repair_and_fallback_store_audit_events` — first invalid output records repair request; second records unstructured fallback with raw text.
- `delegation_rejections_are_inspectable_and_not_misleading` — depth/fanout reason and rejected count are persisted and display text matches execution.
- `unclosed_machine_fence_preserves_text` — stripping is lossless when no closing fence exists.
- `unsupported_schema_version_precedes_unknown_field_error` — request and result parsers return the version error deterministically.

## Integration / Functional Tests

- Re-run all contracts from PRs #14–#16 after fixes.
- Full Rust/Vitest/check/build suites pass.
- Archive integration uses a real temporary SQLite forest with sessions, entries, heads, knowledge, leases, ledger, and legacy events.
- Repair integration exercises the same helper called by `forward_turn_result`, including stored audit rows.

## Smoke Tests

- `bun run test` passes.
- `bun run check` passes.
- `bun run build` passes.

## E2E Tests

- Compatibility event → dual-write → active forest replay succeeds.
- Malformed worker output → same-session repair audit → malformed repair → unstructured audit/result succeeds.
- Clean stopped workspace with forest history → archive succeeds with no orphan rows.

## Manual / cURL Tests

- N/A for cURL — local Tauri/SQLite behavior only.
- Confirm raw worker-result JSON remains in `agent_events` while `reduceConversation` omits it from visible text.
- Confirm depth/fanout rejection reason events are present in normalized history.
