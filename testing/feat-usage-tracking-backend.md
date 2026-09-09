# Contract: token and cost usage tracking — backend

Branch `feat/usage-tracking-backend` (history importers land on
`feat/usage-history-import` and merge into it). Backend half of #584.
Locked before implementation. The frontend is a separate follow-up.

## What this changes

Bridge already writes one `usage_ledger` row per provider `usage.updated`
event, but the rows are uneven: Codex rows carry tokens and no cost, Claude
rows carry an aggregate turn cost but lose the per-model split of multi-model
turns, OpenCode rows lose `cost` entirely (the key is not one the ledger
reads), and Cursor/Grok rows carry a context gauge only. Nothing prices
unpriced rows, nothing rolls rows up by day or hour, and the
`agent_usage_*` analytics tables from migration 44 have no importer, so usage
from sessions Bridge did not run is invisible.

This change gives Bridge the same model as T3 Code's usage service, in Rust,
and goes further where T3 Code is weaker:

1. **Normalized per-request usage** with mutually exclusive token buckets
   (`uncached_input`, `cache_read`, `cache_write`, `output`, `reasoning`),
   the serving model, the model context window, and a provider record id.
2. **Pricing** from a bundled rate table (LiteLLM-derived, base tier only),
   with per-model user overrides, an explicit refresh method, and a
   `cost_source` of `provider_reported | model_priced | unpriced` on every
   row. Cache savings are computed alongside cost.
3. **Aggregation** into day or hour buckets per harness and model, exposed
   as a new wire method, with the weakest cost provenance in a bucket
   winning and imported and live records de-duplicated against each other.
4. **History importers** for Claude Code (`~/.claude/projects/**/*.jsonl`),
   Codex (`~/.codex/sessions/**/*.jsonl`), and OpenCode (`opencode.db`),
   writing exact provider-reported observations into `agent_usage_*` with
   incremental per-file cursors. Cursor is discovered and reported as
   `unsupported` with a reason: its local stores hold no token counts.

## Non-negotiable honesty rules

1. **Never invent a number.** A bucket with no rate is `unpriced` and
   contributes zero cost while still counting tokens. A provider that reports
   no tokens yields no token figures, not zero.
2. **Cost provenance is per row and per bucket.** A bucket mixing reported
   and priced rows is `model_priced`; a bucket of only unpriced rows is
   `unpriced`. `provider_reported` requires every row to carry a provider
   cost.
3. **No double counting.** Codex cumulative `tokenUsage.total` never enters
   a ledger row (only `last`). Claude JSONL repeats are dropped by
   `message.id:requestId`. Codex rollout duplicates are dropped by identical
   consecutive `last_token_usage`; fork/subagent rollouts drop the copied
   parent burst (gap < 1 s from `session_meta`). Live and imported records
   for the same provider session count once: when a Bridge session's
   `provider_session_id` matches an imported `native_session_id`, the
   summary takes the imported observations (the transcript is the complete
   record) and drops that session's live rows, reporting them in
   `duplicatesDropped`. Claude rows additionally carry
   `provider_record_id = message.id:requestId` for record-level checks.
4. **Reasoning tokens are a breakdown, not an addend.** They are already
   inside `output_tokens` for every supported provider and are never priced
   or totalled separately.
5. **Cache-inclusive input is provider-specific.** Codex and OpenAI-style
   `input_tokens` include the cached portion; Anthropic's exclude it. The
   normalizer applies the documented formula per provider
   (`analytics::ExactTotalFormula`) and never a generic one.
6. **Rates are base tier.** Long-context tiers (`[1m]`, `>200k`) and
   service tiers are not known per request, so they are not priced; the
   variant suffix is stripped and the base rate used, and the bucket says so
   through `pricing.status` and the row's `cost_source`.
7. **No ambient network.** The bundled rate table ships in the binary. A
   fresh LiteLLM table is fetched only by the explicit `usage/refresh_rates`
   method, cached under the data directory with its fetch time, and
   `pricing.status` reports `bundled | fresh | cached | unavailable`.
8. **Bare family names are unpriceable.** `opus`, `sonnet`, `haiku`,
   `fable`, `<synthetic>` never resolve to a rate.
9. **Importers read numbers only.** No prompt or completion text crosses
   into Bridge's store; observations carry token counts, model, timestamps,
   ids, and cost.

## Provider mapping (live path)

| Provider | Source event | Tokens | Cost | Model | Notes |
| --- | --- | --- | --- | --- | --- |
| Codex | `thread/tokenUsage/updated` → `last` | input (cache-inclusive), cached, cache-write, output, reasoning | none → `model_priced` | requested model, or `model/rerouted` serving model when observed | `modelContextWindow` recorded |
| Claude | `result.usage` + `result.modelUsage` | per model: input (exclusive), cache read, cache creation, output | `total_cost_usd` → `provider_reported`; per-model `costUSD` when present | each key of `modelUsage`; falls back to session model | one row per model in the turn; per-model `contextWindow` recorded |
| OpenCode | `step-finish` part `tokens` + `cost` | input (cache-inclusive), cache read/write, output, reasoning | `cost` → `provider_reported` | `modelID` / `providerID` from the message | `cost` key must be read; today it is dropped |
| Cursor / Grok (ACP) | `usage_update` gauge | none | ACP `cost` only when USD | session model | `context_percent` only; rows are `unpriced` unless a cost arrives |

## Schema

`usage_ledger` gains nullable columns: `reasoning_tokens`, `serving_model`,
`context_window_tokens`, `context_used_tokens`, `provider_record_id`,
`cache_savings_microusd`. New table `usage_price_overrides(model TEXT PK,
input_microusd_per_mtok, output_microusd_per_mtok, cache_read_microusd_per_mtok,
cache_write_microusd_per_mtok, updated_at)`. New table
`usage_rate_cache(id INTEGER PK CHECK(id=1), fetched_at, source_url, body)`.
`agent_usage_sources.scan_cursor` (already present) holds JSON per-file
positions. Existing rows keep every current value; migration is additive.

## Wire methods (bridge-protocol)

- `usage/summary` — params `{ sinceDay, untilDay, resolution: "day"|"hour",
  timeZone?, workspaceId?, includeImported: bool }`; result
  `{ buckets[], sources[], pricing, scanDurationMs, duplicatesDropped }`.
  Bucket: `{ day, hourStart?, harness, model, totals{uncachedInputTokens,
  cacheReadTokens, cacheWriteTokens, outputTokens, reasoningTokens},
  costMicrousd, cacheSavingsMicrousd, costSource, records, unpricedRecords,
  sessions }`.
- `usage/list_price_overrides`, `usage/set_price_override`,
  `usage/clear_price_override` — user rates in micro-USD per million tokens.
- `usage/refresh_rates` — explicit fetch; result is the new `pricing` block.
- `usage/list_history_sources` — discovered importers with capability,
  coverage, last scan, reason.
- `usage/scan_history` — params `{ maxRecords?, sourceIds? }`; bounded,
  incremental; result `{ sources[], recordsImported, recordsSkipped,
  durationMs }`.

All params carry `deny_unknown_fields`. Artifacts are regenerated; the
mirror, tsgen, and registry↔handler gates stay green.

## Unit tests (Rust, inline `mod tests`)

### usage normalization (`usage.rs`)
- Codex `last` slice with cache-inclusive input yields
  `uncached = input − cached − cache_write`, never negative; reasoning is
  clamped to output.
- Codex frames with only `total` produce no row.
- Claude `result` with `modelUsage` for two models produces two rows with
  per-model tokens and the turn cost attributed once (not duplicated per
  row); the aggregate `usage` is not written as a third row.
- Claude `result` without `modelUsage` produces one row from `usage`.
- OpenCode `step-finish` yields tokens and `cost` → `provider_reported`,
  model from the owning message.
- ACP `usage_update` yields `context_percent` and no token figures; a
  non-USD `cost` is ignored.
- Serving model from Codex `model/rerouted` overrides the requested model
  for subsequent rows in that turn only.

### pricing (`usage_pricing.rs`)
- Bundled table parses, every entry has input and output rates.
- `lookup_rate` strips `[1m]`, lowercases, drops provider prefix when the
  bare name is unambiguous, refuses bare family names and `<synthetic>`.
- `price` returns `provider_reported` when a cost is reported and no
  override exists; `model_priced` from the table; `unpriced` when neither.
- An override beats a reported cost (matches T3 Code semantics) and beats
  the table.
- `cache_savings` is `cache_read × (input_rate − cache_read_rate)`, zero
  when unpriced.
- Micro-USD arithmetic uses integer math; a 1 M-token input at $3/M is
  exactly 3 000 000 µUSD.

### aggregation (`usage_summary.rs`)
- Day buckets respect the caller's time zone boundary (a 23:30 UTC-5 record
  lands on the local day).
- Hour resolution requires exact bounds and rejects a window over 24 h.
- Weakest-provenance rule for `costSource` across mixed rows.
- A Bridge session whose `provider_session_id` matches an imported
  `native_session_id` contributes only its imported observations;
  `duplicatesDropped` counts the live rows set aside. A session with no
  imported match keeps its live rows.
- Buckets sort by day, hour, harness, model; output is deterministic.
- `sessions` counts distinct session ids that contributed.

### importers (`usage_import/*.rs`)
- Claude: assistant records only; `message.id:requestId` dedupe keeps the
  first; `<synthetic>` model is skipped; `costUSD` when present becomes
  `provider_reported`; `isSidechain` records are imported and attributed to
  the parent session.
- Codex: model carried from `turn_context`; `token_count` before any
  `turn_context` is skipped without consuming the duplicate signature;
  identical consecutive `last_token_usage` dropped; fork burst suppressed;
  `session_meta` after the first is ignored.
- OpenCode: reads `message.data` for `role=assistant` with `tokens`;
  `cost` is `provider_reported`; `modelID` and `providerID` recorded;
  cache read/write mapped; input treated as cache-inclusive.
- Cursor: discovery returns `unsupported` with a reason and imports nothing.
- Incremental scan: a second pass over an unchanged file imports zero
  records; an appended file imports only the new lines; a truncated or
  rewritten file (guard hash mismatch) rescans from the start.
- Scan batches respect `max_records` and return a cursor.

### store
- Migration adds the new columns and tables idempotently; existing
  `usage_ledger` rows survive with prior values.
- `agent_usage_observations` unique `(source_id, native_record_id)` holds.

## Integration tests

- `bridged/tests`: `usage/summary` over a seeded database returns buckets
  with correct totals, cost, and provenance for one Codex, one Claude, and
  one OpenCode session; `includeImported=false` excludes observations.
- `protocol_mirror::result_payloads_mirror_core` covers every new result
  type; `tsgen::checked_in_artifacts_match_the_contract` passes after
  regeneration.
- `src-tauri/src/lib.rs` registry↔handler pin includes every `usage/*`
  method.

## Smoke

- `cargo test --workspace` green; `bun run check` green (generated TS
  compiles; no frontend consumer yet).
- `bridge exec --json usage/summary` against a real data directory returns
  buckets for the last 7 days without panicking on a provider with no rows.
- `usage/scan_history` against this machine's `~/.claude/projects`,
  `~/.codex/sessions`, and `~/.local/share/opencode/opencode.db` imports a
  non-zero record count, and a second run imports zero.

## Out of scope

Frontend usage page, charts, and settings UI; subscription-quota windows
(already handled by `account-usage`); billing tiers above base; Grok
transcript import; Cursor token import (no local source exists).

## PR review verification (2026-09-09)

- Reviewed the complete `67fe4450..HEAD` merge-base diff. Confirmed fixes:
  OpenCode rescans now replace stable message ids, importer-version changes
  persist their cleared cursor/version before a failed retry, and incomplete
  rebuilds do not suppress complete live-session totals.
- Actual-scan regressions cover an updated OpenCode message and a version
  reset followed by failure, retry, and two bounded rebuild batches.
- Isolated daemon API QA under `/tmp/bridge-588-qa.d0uR0x` imported one
  OpenCode row, replaced it in place after an edit, returned the updated
  model/tokens/provider cost in `usage/summary`, repriced a seeded live row to
  158 µUSD via an override, round-tripped and cleared the override, and
  rejected a negative rate.
- `bun run build` passed. The full test command passed with the host's
  intentional cache variable removed:
  `env -u CARGO_TARGET_DIR NODE_OPTIONS=--no-experimental-webstorage bun run test`
  (44 sidecar tests passed, 1 authenticated-only skip; 1,862 Vitest tests
  passed; the Rust workspace and doc tests passed with only documented
  live/network tests ignored).

## Addendum: repair of rows written before the normalizers

Locked before the fix branch `fix/usage-summary-honesty`. Live ledgers from
before this contract hold two families of rows that recorded a provider's
running total as one turn's figure, and the summary summed them:

1. **Claude cost.** The SDK documents `total_cost_usd` and `modelUsage` as
   cumulative across the turns of one process ("read the latest result
   rather than summing"). The Claude stream state now keeps the previous
   result's totals and emits each result as its difference; a total that goes
   backwards (a resume, a `/clear`, a restarted process) is a fresh run and is
   taken as-is; `system/init` resets the totals. Migration 54 rewrites
   existing Claude rows to the same per-turn difference, per session and
   model. On the reference machine this moved 30 days of Claude live cost
   from $12,221 to $1,331, which is what the last frame of each session said.
2. **Codex cumulative tokens.** The adapter that read `tokenUsage.total`
   wrote the thread's running totals, but existing rows have no reliable
   cumulative/per-request discriminator. Migration 54 must leave all Codex
   rows unchanged. Neither increasing counts, absent cache fields, nor a
   wall-clock cutoff proves which adapter wrote a row. The earlier timestamp
   repair was unsafe and is removed; immutable provider history supplies
   per-request observations instead. This does not restore rows in databases
   that already ran the old migration. No new speculative repair is allowed.
   Regression coverage includes monotonic, cache-less rows before the old
   cutoff as well as newer and unparsable timestamps.
3. **Unknown harness.** A live row with a null harness is attributed to the
   provider named in its `source` (`provider.codex` is Codex), never to
   `unknown`.
4. **Unknown cache split.** A cache-inclusive provider's row with input but no
   cache figure at all cannot be priced honestly: cached input is a tenth of
   the rate and most of an agentic request. Such rows count their tokens and
   stay `unpriced`. Anthropic rows are exempt, since their input is exclusive
   and a missing cache figure is a zero.

Validation against T3 Code on the same machine and window (Aug 11 to Sep 9,
Asia/Kolkata), after a full history scan: Claude $5,649 / 7.76B tokens
against T3's $5,455 / 7.61B; `claude-fable-5` matches to the cent; Codex $880 /
1.45B against T3's $739 / 1.30B. The remainder is live rows for sessions that
have no rollout on disk, which T3 cannot see, and a rate-snapshot difference on
`gpt-5.6-sol`. Imported Codex `gpt-5.6-sol` alone is 959M tokens, exactly T3's
figure.
