# Memory paging and bulk review contract

Issue #1145, part of epic #1144. Locked before implementation.

## Functional behavior and tests

- `memory/list_memory_records` takes optional `cursor` and `limit` and returns
  `total` and `nextCursor`. Omitting both returns the same first 50 records as
  before, in the same order (`updated_at DESC, id DESC`).
  Test: 138 proposals page as 50, 50, 38 with no duplicate and no gap, and
  `nextCursor` is absent on the last page. `total` is 138 on every page.
- Paging is keyset, so it stays correct while records change. Test: records that
  share an `updated_at` page across the boundary without loss. Test: approving a
  record on page 1 after reading it neither skips nor repeats a record on page 2.
- `limit` outside 1..=50 and a malformed cursor are `Invalid` errors, never a
  silent first page. Test both, plus a cursor from another status.
- `memory/review_memory_records` approves and rejects a batch in one
  transaction and publishes one `MemoryChanged`. At most 200 ids. An id in both
  lists is `Invalid` and changes nothing. Test: a batch with 3 approvals and 2
  rejections changes those 5 records and reports them.
- An id that is not a proposal (already reviewed, unknown) goes to `refused`
  with a reason, and the rest of the batch still applies. Test it.
- Approval goes through the same ledger path as `approve_memory_record`, so the
  budget rule M3 adds applies to bulk approval without new code here.
- Memory dialog: tab badges show `total` (138), not the loaded count (50).
  "Show more" appends the next page. Test with a mocked 138-proposal queue.
- Memory dialog: when a search or kind filter is active and more active pages
  exist, the remaining pages load, so search never misses an unloaded record.
- Memory dialog: a checkbox per proposal, "Select all shown", and "Approve
  selected" / "Reject selected" call `reviewMemoryRecords` once. Selection
  clears afterwards and refusals are reported. Test it.
- A `MemoryChanged` reload keeps as many records loaded as before, so a bulk
  action does not throw the user back to page 1.
- The mock backend in `api.ts` pages the same way, so `bun run dev` shows it.

## Verification

- Focused: `cargo test -p bridge-core memory_ledger::`, `-p bridge-protocol`,
  `bunx vitest run src/components/MemoryDialog.test.tsx src/api.test.ts`.
- Regenerate protocol artifacts from the worktree root and run the drift tests.
- `bun run check`, `bun run test`, `bun run build` before the PR.
- Screenshots of the Review tab (dark and light) in the PR body.
