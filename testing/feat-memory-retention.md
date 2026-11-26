# Memory retention and budget split contract

Issue #1147, part of epic #1144. Locked before implementation.

## Functional behavior and tests

- Budget split: the active budget (`max_records`, default 200) counts only
  `active` records. Proposals have their own cap (`max_proposals`, default 100).
  Test: 138 proposals and 10 active records do not block a save or an approval.
- Approval checks the active budget, singly and in a batch. Test: bulk approve of
  60 proposals at 190 of 200 active approves 10 and refuses 50 with a budget
  reason, in one call.
- At the proposal cap, a new proposal is refused, and extraction's precheck
  skips the run (`scope_full` detail) without spending. Active space does not
  matter for that check.
- Proposal TTL is a setting: 3, 7, 14 or 30 days, default 7. Test the boundary:
  a proposal exactly N days old expires, and one a second younger does not.
- `last_used_at` is stamped when the packet selects a record and when
  `memory_search` returns it. The migration backfills it from
  `memory_retrieval_audits`.
- Unused retirement: an active record that is not `user_explicit` and whose
  `COALESCE(last_used_at, created_at)` is at least N days old moves to `expired`
  with `valid_to = now`. N is 30, 90 or 180, or never; default 90. Test: a pin
  unused for a year is never retired; with "never", nothing is retired.
- Each sweep that changes something writes one `memory_retention_sweeps` row
  (kind, count, ids). `get_activity_log` lists them as source `retention`.
- Restore: `memory/restore_memory_record` turns an expired record that once held
  (not a proposal) into a new active record that supersedes it, keeping body,
  kind and provenance. It checks the active budget and stamps `last_used_at`.
  Test: the restored record is selected by the packet again; restoring at a full
  budget is refused; restoring an active record or an expired proposal is refused.
- `list_memory_records` accepts `status: "expired"`, paged like the others.
- Settings round-trip through `get_retention_settings` / `set_retention_settings`
  with validation. Invalid values change nothing.
- UI: a "Clean up" card in Review sets proposal expiry and retire-after. The
  budget banner counts active memories only, and the queue explains its own cap.
  A Retired list under Memories pages expired records with Restore. Activity
  shows retention sweeps.

## Verification

- Focused: `cargo test -p bridge-core memory_`, `-p bridge-protocol`, store
  migration tests, `bunx vitest run src/components/MemoryDialog.test.tsx src/api.test.ts`.
- Regenerate protocol artifacts; `bun run check`, `bun run build`, vitest,
  `cargo test --workspace --no-fail-fast`, the clippy ratchet, `check:visual`.
- Screenshots dark and light, and one with Restore.
