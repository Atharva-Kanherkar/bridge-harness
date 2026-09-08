# fix-cache-telemetry-codex-alias — Test Contract

Locked before implementation. Branch cut from freshly fetched `origin/main` at
`a8292c2bbefdc06ecf91d182341d0aafcc311ac7`.

Implements issue #526 (`[1/6]` of epic #525, gap **G7**). Scope is telemetry
only: nothing here changes prompt delivery, session switching, restoration, or
compaction arming.

The Codex wire shape below is taken from the app-server's own generated schema
(`codex app-server generate-json-schema`, codex-cli 0.153.4), not inferred:

```
thread/tokenUsage/updated
  { threadId, turnId, tokenUsage: { total: Breakdown, last: Breakdown, modelContextWindow: int|null } }
  Breakdown = { totalTokens, inputTokens, cachedInputTokens, cacheWriteInputTokens,
                outputTokens, reasoningOutputTokens }
```

`serde_json` runs with `preserve_order` enabled (pulled in by `schemars`), so
`Value::Object` preserves wire order and `integer_alias`'s recursive descent
resolves against `tokenUsage.total` — declared first — rather than a per-request
figure. That is the defect this contract closes.

## Functional Behavior

### 1. Codex token usage is normalized at the adapter boundary

- `thread/tokenUsage/updated` produces a `usage.updated` event whose `data`
  carries an explicit `usage` object, the same shape Claude and OpenCode already
  emit. Downstream consumers never depend on recursive alias descent to find
  Codex numbers.
- The `usage` object is built from `tokenUsage.last` — the most recent model
  request — mapping `inputTokens`, `cachedInputTokens`, `cacheWriteInputTokens`,
  `outputTokens`, `reasoningOutputTokens`, and `totalTokens`.
- The raw notification body, including `tokenUsage.total` and
  `modelContextWindow`, is preserved alongside the normalized `usage` object.
  Nothing that reads the running total or the context window loses access to it.
- A notification that omits `tokenUsage.last` yields no fabricated numbers.

### 2. A usage row is a per-request delta, not a thread-cumulative total

- One `usage_ledger` row per usage frame records that request's own token
  counts. Summing a turn's rows yields the tokens that turn actually consumed.
- Before this change a single Codex turn wrote 54 rows whose `input_tokens`
  climbed monotonically from 2,016,065 to 8,893,494 — the same cumulative
  counter re-recorded per frame, and `uncached_input_tokens` mirrored it because
  both cache figures were NULL. That is the behavior being corrected.
- Rows written before this change are cumulative and are **not** backfilled or
  migrated. The discontinuity is documented rather than papered over; any
  aggregate spanning the change is not comparable.

### 3. Cache alias vocabulary covers camelCase providers

- The cache-read alias list resolves `cachedInputTokens` in addition to the
  existing `cache_read_tokens`, `cacheReadTokens`, `cached_input_tokens`, and
  `cache_read_input_tokens`.
- The cache-write alias list resolves `cacheWriteInputTokens` in addition to the
  existing `cache_write_tokens`, `cacheWriteTokens`, and
  `cache_creation_input_tokens`.
- Claude's existing snake_case resolution is unchanged.

### 4. OpenCode reports cache writes as well as cache reads

- The OpenCode `message.updated` normalizer maps `tokens.cache.write` to a cache
  write figure alongside the existing `tokens.cache.read` mapping, so write
  amortization and hit ratio become computable for OpenCode rows.
- An OpenCode payload with no `cache.write` records no cache write rather than a
  zero.

### 5. Every provider usage row carries harness and model

- `record_provider_usage` stamps `harness` and `model` from the matching
  `prompt_compilations` row when one exists for the turn, exactly as today.
- When no compilation matches the turn — the common case, because a compilation
  binds to a single turn per launch while a session runs many turns — `harness`
  and `model` fall back to the `sessions` row.
- No provider usage row is written with `harness IS NULL` while a `sessions` row
  for that session exists.
- Dimensions that only a compilation can supply — `stable_prefix_id`,
  `stable_prefix_hash`, `prompt_schema_version`, `prefix_token_estimate`,
  `role`, `task_family`, `restoration_mode`, `cross_harness_reuse` — remain
  compilation-only. They are never invented from the session row.

### 6. Root chats record real cross-harness reuse

- A root-chat prompt compilation resolves `cross_harness_reuse` against the
  session's previous compilation: same harness → `same_harness`, different
  harness → `incompatible`, no previous compilation → `not_applicable`.
- The existing three-value vocabulary is reused, so no consumer — including
  `buildCacheDiagnostics` — needs to learn a new marker.
- The worker path (`cross_harness_reuse_marker` against the parent session) is
  unchanged.

### 7. Uncached input derivation stays correct per provider

- For Codex, `inputTokens` is inclusive of cached and cache-write tokens, so
  uncached input is `input - cache_read - cache_write`, floored at zero. This is
  the existing non-Claude branch; it now operates on real cache figures instead
  of NULLs.
- Claude's special case — where `input_tokens` already excludes cache tokens —
  is unchanged.

### 8. Explicitly out of scope

- `context_percent` is **not** synthesized for Codex, even though
  `modelContextWindow` is now reachable. Populating it would newly arm
  `begin_pressure_compaction` for Codex sessions, which is a behavior change
  belonging to #530 (gap G6). No pressure-compaction gate moves in this change.
- No change to prompt assembly, delivery, model switching, restoration mode
  selection, or checkpointing.
- No protocol/wire-schema change: `usage_ledger` columns and the generated
  protocol artifacts are untouched.

## Unit Tests

Rust (`src-tauri/bridge-core`):

- `agent::tests` — a real-shaped `thread/tokenUsage/updated` notification
  normalizes to one `usage.updated` whose `data.usage` reflects `tokenUsage.last`
  and **not** `tokenUsage.total`. The fixture uses distinct values for `last` and
  `total` so a regression to the cumulative counter fails the assertion.
- `agent::tests` — the same event retains the raw `tokenUsage` object including
  `total` and `modelContextWindow`.
- `agent::tests` — a `thread/tokenUsage/updated` without `tokenUsage.last`
  produces no invented figures.
- `agent::tests` — OpenCode `message.updated` with `tokens.cache.write` maps a
  cache write; without it, none.
- `policy::tests` — `UsageReport::from_normalized` over the normalized Codex
  event resolves input, output, cache read, and cache write to the per-request
  figures.
- `policy::tests` — alias coverage: a bare `{"usage": {"cachedInputTokens": N,
  "cacheWriteInputTokens": M}}` resolves both.
- `policy::tests` — `record_provider_usage` stamps `harness` from the `sessions`
  row when no compilation matches the turn, and a matching compilation still
  wins over the session row.
- `policy::tests` — uncached input for a Codex row equals
  `input - cache_read - cache_write` and never goes negative.
- `live_turn::tests` — a root-chat compilation records `same_harness` when the
  previous compilation used the same harness, `incompatible` when it differed,
  and `not_applicable` on the session's first compilation.

Frontend (Vitest):

- `src/usage.test.ts` — `buildCacheDiagnostics` produces a Codex group with a
  real `cacheHitRatio` from reported reads and writes, not a 0% ratio derived
  from uncached input alone.
- `src/usage.test.ts` — an OpenCode row with both reads and writes yields a
  defined `writeAmortization`.
- `src/usage.test.ts` — `extractUsageSnapshot` returns total tokens for a
  normalized Codex `usage.updated` payload, which it cannot do today because it
  reads only `data.usage`.

## Integration / Functional Tests

- `builtin_compatibility` replay: `testing/fixtures/builtin-adapter-events-v1.json`
  carries the real nested `thread/tokenUsage/updated` shape in place of the
  invented flat `{inputTokens, outputTokens}` body, and its expected event
  asserts the normalized per-request usage. The replay stays green.
- A provider usage event for a session with no bound compilation appends a
  ledger row with `harness` populated and compilation-only dimensions NULL.

## Smoke Tests

- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core policy::`
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core agent::`
- `bunx vitest run src/usage.test.ts`
- `bun run check`
- `bun run build`
- `bun run test`

Rust failures are diffed against the known pre-existing `main` adapter failures;
only a delta counts as a regression.

## E2E Tests

N/A automated — this change has no user-driven journey. The dev-build check
under Manual replaces it.

## Manual / cURL Tests

On a dev build (`bun run tauri dev`) against a scratch `BRIDGE_DATA_DIR`, run one
Codex turn, then:

```sql
-- must be non-zero
SELECT count(*) FROM usage_ledger
WHERE harness='codex' AND cache_read_tokens IS NOT NULL;

-- must be zero
SELECT count(*) FROM usage_ledger
WHERE source LIKE 'provider.%' AND harness IS NULL;

-- per-request, not cumulative: input_tokens must not climb monotonically
SELECT id, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
       uncached_input_tokens
FROM usage_ledger WHERE source='provider.codex' ORDER BY id;

-- root-chat switches are now visible
SELECT harness, cross_harness_reuse, count(*) FROM prompt_compilations
GROUP BY 1,2;
```

Then open the usage widget, expand **Show more**, and confirm the **Prompt cache**
section lists a Codex group whose hit ratio is a real reported figure rather than
0%. Verify `cachedInputTokens + cacheWriteInputTokens <= inputTokens` holds in
the captured frame; if a real capture contradicts it, the §7 derivation is
revisited and this contract is amended before the code is.
