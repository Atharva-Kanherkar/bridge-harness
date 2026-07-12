# codex/issue-6-capability-tiers — Test Contract

## Functional Behavior

- `fast`, `standard`, and `strong` are the only durable routing tiers shared by Rust models, adapter inventory, typed delegation, API types, and UI copy.
- Every advertised provider model belongs to exactly one tier, and each adapter advertises one deterministic default model per supported tier.
- Runtime worker launch resolves the requested tier through the selected installed adapter. A known compatibility model hint is accepted only when it belongs to the requested tier.
- Unknown or tier-mismatched model hints fall back to the adapter's tier default and append a queryable warning event.
- Worker sessions persist `requested_tier` independently from the actual provider `model`; spawn events expose both.
- Existing databases migrate transactionally and idempotently to the new session audit column.
- The orchestrator briefing uses role/tier/effort and the typed v1 request envelope, preserves the rule against delegating trivial actions, enforces flat topology, and contains no provider/model branding as routing guidance.
- The orchestrator relays typed worker-result summaries and never asks for or forwards raw worker transcripts.
- UI session copy presents tier as the routing semantic and actual model as secondary runtime detail; user-facing policy copy does not name a routing model.

## Unit Tests

- Every model in every adapter descriptor maps to exactly one tier and tier defaults point to advertised models.
- Tier resolution is deterministic for known hints, absent hints, unknown hints, and hints from the wrong tier.
- Unknown/tier-mismatched resolution reports a fallback warning.
- Orchestrator briefing snapshot assertions cover all roles, tiers, efforts, typed request/result vocabulary, flat topology, trivial local action handling, and absence of advertised provider model IDs/labels.
- Migration tests prove v2 → v3 adds `requested_tier`, preserves existing session models, and is idempotent.
- Runtime reservation test proves requested tier and resolved actual model are both persisted and included in the spawn audit payload.

## Integration / Functional Tests

- Parent typed request with `capabilityTier=strong` resolves through adapter inventory before process start, persists tier + model, and passes the resolved model to the adapter.
- An unknown compatibility model hint produces a safe tier-default model and a warning event without bypassing policy.
- Existing policy limits and typed-envelope validation remain unchanged.

## Smoke Tests

- `bun run test` passes.
- `bun run check` passes.
- `bun run build` passes.
- `git diff --check origin/main...HEAD` passes.

## Manual Audit

- Grep routing/policy/briefing code for advertised provider model IDs and labels; matches are limited to adapter inventory, normalization/display compatibility, fixtures, and runtime detail UI.
- Verify no raw-transcript instruction remains in the orchestrator briefing.
