# codex/issue-81-cache-aware-prompts — Test Contract

## Functional Behavior

- Compile a versioned prompt into a byte-stable invariant prefix followed by a bounded variable suffix.
- Stable content includes role instructions, sorted tool schemas, and sorted project rules; variable content includes task facts, selected evidence, timestamps, user input, and session capability references.
- Identical invariant inputs produce the same prefix ID, content hash, schema version, bytes, and token estimate.
- Variable-only changes do not alter the prefix ID or hash; invariant tool or rule changes do.
- Secret-shaped values and session-specific capability references are rejected from the reusable prefix and remain only in the variable suffix.
- Codex and Claude receive the stable prefix before variable/restoration content.
- Usage rows persist normalized cache read, write, and uncached-input tokens together with prefix identity, schema version, cost source, harness, model, role, task family, restoration mode, and cross-harness reuse status.
- Cache diagnostics report hit ratio and write amortization by harness and model, and never claim monetary savings when provider cost is absent.

## Unit Tests

- `prompt_compiler` snapshot tests cover stable ordering, byte stability, hash invalidation, variable isolation, bounds, and secret exclusion.
- Provider cache normalization covers reported reads/writes, derived uncached input, missing metrics, zero denominators, and cost-source preservation.
- Store migrations and round trips preserve all prefix/cache dimensions and remain idempotent.
- Frontend aggregation tests cover harness/model grouping, hit ratio, write ratio, cross-harness markers, and unknown cost.

## Integration / Functional Tests

- Codex thread parameters and Claude sidecar options place compiled stable content before variable content.
- Provider usage events append a fully dimensioned usage-ledger row when session metadata is available.
- Cross-harness checkpoint restoration records whether stable-prefix reuse is compatible.
- Exported Bridge state includes cache diagnostics derived only from recorded provider metrics.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds.
- Current-schema and upgraded-schema databases have identical cache telemetry columns.

## E2E Tests

- A deterministic adapter-level journey compiles the same invariant prompt with two different user inputs and proves identical prefix identity plus suffix-only variation for both Codex and Claude payloads.

## Manual / cURL Tests

- Inspect an exported state snapshot and confirm cache diagnostics identify harness/model/prefix without exposing prefix contents or secret/capability values.
- Confirm diagnostics with missing provider cost show token efficiency only and no fabricated savings amount.
