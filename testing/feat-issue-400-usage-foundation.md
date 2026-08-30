# feat/issue-400-usage-foundation — Test Contract

## Functional Behavior

- A separate global analytics schema exists alongside `usage_ledger`; deleting a
  workspace must not delete indexed whole-Mac analytics metadata.
- An observation records only numeric token/cost fields and bounded provenance.
  It has no field that can hold prompt, response, tool, or transcript content.
- Exact totals are provider-reported totals or a provider adapter's explicit,
  mutually exclusive arithmetic. Unknown totals remain unknown.
- Provider adapters declare their support and coverage status explicitly. A
  linked but unsupported agent is never represented as zero usage.
- A source scan can report records imported/skipped, a cursor, warnings, and
  progress without exposing source-file content.

## Unit Tests

- `analytics::TokenUsage::exact_total` preserves a provider-reported total and
  derives only documented exact formulas.
- `analytics::TokenUsage::exact_total` leaves incomplete or inconsistent token
  buckets unknown rather than estimating them.
- `analytics::AnalyticsImporter` contract rejects an unsupported importer from
  claiming complete coverage.
- Analytics record validation refuses sensitive JSON keys and values from the
  small forward-compatible numeric payload.

## Integration / Functional Tests

- Opening a database migrates it to the analytics schema with sources,
  sessions, observations, and attributions tables and dedupe indexes.
- The legacy `usage_ledger` remains independent of the global analytics
  tables.

## Smoke Tests

- `cargo test -p bridge-core analytics` passes.
- `bun run build` completes.

## E2E Tests

N/A — no importer or dashboard UI is introduced in this foundation slice.

## Manual / cURL Tests

N/A — the new surface is internal Rust/SQLite only; protocol commands are
introduced with the importer and query slices.
