# Contract: repair Claude ledger rows written as running totals

Branch `fix/claude-usage-ledger-backfill`. Schema migration 87,
`store::migration_87_claude_running_total_repair`.

## The bug being repaired

Migration 54 shipped with Claude usage code that stored each `result` as its
difference from the one before, but cleared the previous totals at every
`system/init`. The SDK sends `init` every turn, so every Claude `usage_ledger`
row written by that code holds the session's totals so far. Every usage view
sums rows, so a long chat is counted once per turn. On the author's database,
Nov 20 read 13.8B Claude tokens in the ledger against 226M in Claude's own
transcripts. The code that keeps totals across `init` and seeds them on a
resume fixes new rows; this contract covers the rows already written.

## Expectations

1. Rows in scope are `source='provider.claude'`, written at or after
   `schema_version.applied_at` for version 54 on this install.
2. Per session, rows at or after the first row with `context_used_tokens`
   (only the fixed code sets it on a Claude row) are out of scope, including
   the helper-model rows written with it.
3. Within scope, per (session, model) in id order: when every figure present
   in both rows is at least the previous row's, the row stores the difference.
   When any figure is lower, the totals restarted and the row stays as it is.
4. Figures repaired: input, uncached input, output, cache read, cache write,
   reasoning, cost, cache savings. Context gauges, runtime, model, cost source
   and timestamps are unchanged.
5. Rows before migration 54 are never changed and never seed a difference.
6. Other providers are never changed.
7. A database with no migration 54 record is left alone.
8. The migration runs once (schema version 87) and inside the migration
   transaction, with the usual pre-migration backup.

## Tests

- `store::tests::claude_running_totals_written_since_migration_54_become_per_turn`
  covers 1 to 6.
- `store::tests::claude_running_total_repair_without_a_migration_54_record_changes_nothing`
  covers 7.
- Manual: run the migration on a copy of the author's `usage_ledger` and
  `schema_version` tables, and compare daily Claude totals before and after
  with the imported transcript totals for the same days.
