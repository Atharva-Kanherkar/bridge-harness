# feat-context-lens-5-prompt-accounting — Test Contract

Implements Atharva-Kanherkar/bridge-harness#243 — Context Lens 5/8: persist exact
Bridge prompt accounting. Non-goal (from issue): measuring provider presets,
provider-native history, or provider-owned tool schemas.

## Functional Behavior

Every serialized byte of a Bridge-authored compiled prompt is attributed to an
accounting entry. The wire payload is `stable_prefix + "\n\n" + variable_suffix`
(prompt_compiler.rs:161), so accounting has three regions:

1. **Stable region** — one entry per surviving element of `StableEnvelope`
   (prompt_compiler.rs:56-64): `role`, each `stable_sections` entry, each
   `tool_schemas` entry, each `project_rules` entry. Entries carry kind
   (`role` | `stable_section` | `tool_schema` | `project_rule`), section id/name,
   serialized byte count, and token estimate.
2. **Variable region** — one entry per surviving `NamedText` in
   `VariableEnvelope` (prompt_compiler.rs:66-70).
3. **Envelope overhead** — explicit entries for bytes that belong to no
   content element: stable tag pair + envelope JSON punctuation
   (`<bridge-stable-prompt schema="1">`, `\n`, `</bridge-stable-prompt>`,
   braces/commas/keys not charged to any element), the same for
   `<bridge-variable-context>`, and the `\n\n` separator between regions.

- Attribution method: **leave-one-out delta serialization.** For each element,
  serialize the full envelope once and once with that element removed;
  attributed bytes = difference. This charges JSON escaping, key names,
  quotes, colons, and separators to the exact element that caused them
  without reimplementing serde_json escaping. Envelope overhead entry =
  region length − Σ(element deltas), so sums close exactly.
- Empty/deleted sections produce **no** accounting entries: they are already
  dropped before serialization by `insert_text` (prompt_compiler.rs:185-194)
  and `variable_section` (prompt_compiler.rs:96-105); accounting must consume
  those same post-filter collections.
- Cache identity is untouched: `prefix_id`, `prefix_hash`, `prefix_bytes`,
  and both prefix strings are byte-for-byte identical to pre-change output.
  Accounting is additive metadata only.
- Persistence on `prompt_compilations` per compilation/turn:
  - `sections_json TEXT NULL` — the full ordered accounting entries above
    (camelCase serde, matching repo conventions),
  - `stable_bytes INTEGER NULL` — equals `stable_prefix.len()`,
  - `variable_bytes INTEGER NULL` — equals `variable_suffix.len()` (first time
    the variable suffix size is persisted anywhere),
  - `stable_token_estimate INTEGER NULL`, `variable_token_estimate INTEGER NULL`
    — `bytes.div_ceil(4)`, source-labelled via `token_estimate_source TEXT NULL`
    (value e.g. `bytes_div4_v1`), bounded by existing caps
    (`MAX_VARIABLE_SUFFIX_BYTES` for the variable side).
- Columns are added idempotently via `add_column_if_missing` following
  `migration_18_prompt_cache_telemetry` (store.rs:476-532); legacy rows keep
  NULLs and remain readable.
- Producers thread the new data through `PromptCompilationRecord`
  (model.rs:599-615) from `CompiledPrompt`; all record constructors updated
  (live_turn.rs:454, live_turn.rs:608, tests).

## Unit Tests (src-tauri/bridge-core/src/prompt_compiler.rs)

- `Test accounting_closes_to_the_exact_wire_bytes` — Σ(stable entries +
  stable overhead) == `stable_prefix.len()`; Σ(variable entries + variable
  overhead) == `variable_suffix.len()`; separator entry == 2; grand total ==
  `instructions().len()`; no unattributed remainder.
- `Test json_escaping_is_charged_to_the_causing_element` — an element whose
  text contains `"`, `\`, and newline attributes more bytes than its raw
  `text.len()`.
- `Test empty_and_deleted_sections_produce_no_phantom_entries` — empty
  stable/project-rule/variable inputs yield zero entries of their kind.
- `Test separator_and_tags_are_explicitly_attributed` — overhead entries exist
  for both envelopes and the separator, none negative.
- `Test token_estimates_are_labelled_and_bounded` — estimates are
  `div_ceil(4)`, carry the source label, variable estimate respects the cap.
- `Test prefix_identity_is_unchanged_by_accounting` — hash/id/bytes equal a
  compilation computed as before (guards provider cache reuse).
- All existing tests in the file pass unmodified.

## Integration Tests (src-tauri/bridge-core/src/store.rs, model.rs)

- `PromptCompilationRecord` round-trip: insert then read back returns identical
  `sections_json`, `stable_bytes`, `variable_bytes`, token estimates, and
  source label (extend store.rs:4415-style test).
- Migration: fresh database and upgraded (pre-column) database both expose the
  new columns; extend the upgraded-vs-current parity test (store.rs ~3559)
  to assert column-set equality including the new columns.
- Legacy row with NULL accounting reads back without error; turn-scoped lookup
  (store.rs:2824) surfaces persisted `variable_bytes` per compilation/turn.

## Smoke Tests

- `cargo check` clean; `bun run build` green (tsc -b + vite + cargo).
- `bun run test` green (vitest + cargo test).

## E2E Tests

N/A — no UI or user-journey change; persistence is observability metadata.

## Manual Tests

- After one live turn against a scratch DB:
  `sqlite3 <db> "SELECT stable_bytes, variable_bytes, json_array_length(sections_json) FROM prompt_compilations ORDER BY id DESC LIMIT 1"`
  returns non-null values with ≥1 accounting entry, and
  `stable_bytes + 2 + variable_bytes` equals the stored compilation's total
  attributed bytes.
