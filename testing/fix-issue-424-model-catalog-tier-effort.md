# fix/issue-424-model-catalog-tier-effort — Test Contract

## Functional Behavior

### 1. Claude default model is a single source of truth (Rust)
- `claude_adapter.rs` exports a public constant (e.g. `DEFAULT_MODEL`) whose value is `"sonnet"`.
- `adapters.rs`'s Claude `default_model` field references that same constant instead of a second `"sonnet"` literal.
- `claude_adapter.rs`'s runtime fallback (`unwrap_or(…)`) references the same constant.
- Changing the constant in one place updates both the catalog default and the runtime fallback.

### 2. Active tier + effort + model visible in the session toolbar for every harness
- When a session has `requestedTier`, `effort`, and/or `model`, a label built from `tierRuntimeLabel()` (or equivalent) is rendered in the `SessionToolbar` area for the active primary session.
- The label is visible for repo/orchestrator sessions (where `modelControl` already appears).
- Direct chats also surface the label in the composer area, since their toolbar deliberately omits `modelControl`.
- The label updates reactively when the session's tier/effort/model changes.

### 3. Effort shown in ChatModelControl dropdown
- Each model row in the `ChatModelControl` picker already shows `option.tier`.
- If the session has an active `effort` value, it is displayed alongside the tier badge in the picker or as a secondary line, so users can see effort at a glance while choosing a model.

## Unit Tests

### Rust
- `claude_adapter::DEFAULT_MODEL` equals `"sonnet"`.
- The constant is used in `claude_adapter::start` (the `unwrap_or` site).
- The `Claude` adapter's `descriptor().default_model` equals `Some(DEFAULT_MODEL.into())`.

### Frontend — `src/utils.test.ts`
- Existing `tierRuntimeLabel` tests continue to pass unchanged.

### Frontend — `src/components/SessionToolbar.test.tsx`
- New test: when `tierLabel` prop is provided, it renders as a visible status element.
- Existing tests remain green (the `modelControl` prop path is unchanged).

### Frontend — `src/components/ChatModelControl.test.tsx` (new or extended)
- The dropdown renders `option.tier` for each model row (existing behavior, now tested).
- When an `effort` value is present, the effort badge/text appears in each row or as a header.

## Integration / Functional Tests
N/A — these are presentation-layer and constant-consolidation changes with no cross-service interaction.

## Smoke Tests
- `bun run build` succeeds (TypeScript compiles).
- `bun run test` passes (Vitest green).
- `cargo check --workspace` passes.
- `cargo test --workspace` passes (Rust tests green).

## E2E Tests
N/A — no end-to-end user journey affected; the changes are additive UI and a Rust constant consolidation.

## Manual / cURL Tests
N/A — desktop app; manual verification is through `bun run dev` visual inspection.
