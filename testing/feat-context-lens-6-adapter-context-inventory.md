# feat/context-lens-6-adapter-context-inventory — Test Contract

Context Lens slice 6 of 8. This slice observes provider-owned context at the
adapter boundary. It does not persist observations or add a wire/API surface;
those are deferred to the next slice.

## Functional Behavior

- `bridge-core::context_inventory` defines the five provider-owned segment
  classes: provider base instructions, tool schemas, MCP/dynamic tools,
  skills/plugins, and agent definitions.
- Every observation has exactly one explicit provenance state: provider
  reported, Bridge measured, Bridge estimated, or unavailable. Unavailable
  observations always include a non-empty human-readable reason.
- Inventory scope distinguishes a provider/catalog inventory from segments
  actually presented to a turn. Lifecycle distinguishes session start, session
  resume, and per-turn presentation.
- Claude records the `claude_code` provider preset at its sidecar option
  injection point, plus the explicitly configured MCP servers and plugins.
  Provider-owned schemas and agent definitions that the SDK does not expose are
  unavailable rather than zero.
- Codex records observations where `thread/start` and `thread/resume` parameters
  are assembled. The start-only `instructions` field and resume behavior remain
  distinct. Provider-owned context that app-server does not expose is
  unavailable rather than inferred from aggregate usage.
- OpenCode records catalog/start/resume observations at session creation or
  adoption and actual-turn observations where the per-turn `system` value is
  assembled. Provider-owned context that the HTTP API does not expose is
  unavailable rather than inferred.
- Bridge-authored `PromptCompiler.tool_schemas` is never used as the inventory
  source for provider-owned tool schemas. An empty compiler map therefore cannot
  silently produce a measured zero-tool observation.
- Provider aggregate token totals are not reverse-allocated across segment
  classes. Only values directly reported by a provider, directly measured by
  Bridge at an injection point, or explicitly estimated from a measured value
  are stored.
- Observation names are UTF-8 safely truncated, item counts are capped, and
  credential-shaped material is redacted with `secret_interception` before it
  can enter an observation. Truncation/capping is explicit in the observation.
- Adding a registered built-in adapter without a complete five-class inventory
  contract fails closed in tests.

## Unit Tests

`bridge-core`, `context_inventory` module:

- `provenance_states_serialize_with_camel_case_fields` — reported, measured,
  estimated, and unavailable states round-trip with their intended payloads.
- `unavailable_requires_a_reason` — an empty reason is rejected.
- `names_counts_and_secrets_are_bounded_before_storage` — names truncate on a
  character boundary, counts cap, and credential-shaped material is replaced.
- `every_inventory_requires_exactly_one_observation_per_segment_class` —
  duplicate or missing classes are rejected.
- `estimated_values_require_an_explicit_method` — estimates cannot masquerade
  as measurements.
- `provider_totals_are_not_reverse_allocated` — an aggregate provider token
  total cannot construct invented per-segment observations.

Adapter tests:

- `claude_context_inventory_covers_start_resume_and_per_turn` — all lifecycle
  phases are explicit; configured MCP/plugin catalog entries are bounded and
  actual presentation is not overstated.
- `codex_context_inventory_covers_start_resume_and_per_turn` — start and resume
  parameter differences are reflected and unexposed provider context is
  unavailable with reasons.
- `opencode_context_inventory_covers_start_resume_and_per_turn` — catalog and
  session phases remain distinct from the per-turn system injection point.
- `provider_tool_inventory_does_not_read_prompt_compiler_tool_schemas` — a
  compiler with no tool schemas cannot yield a measured provider-tool zero.
- `every_registered_adapter_has_a_fail_closed_context_inventory_contract` — the
  built-in registry and inventory contract table match exactly, and every
  adapter covers all five classes for start, resume, and per-turn lifecycle.

## Integration / Functional Tests

- Construct the real Claude sidecar configuration, Codex start/resume request
  parameters, and OpenCode per-turn request body through their production
  helpers; verify inventory lifecycle/scope agrees with the payload assembled at
  each injection point.
- Existing adapter launch, resume, permission, prompt compilation, briefing
  policy, and protocol mirror tests remain green.

## Smoke Tests

- `cargo test -p bridge-core` passes from `src-tauri/`.
- `cargo check` passes from `src-tauri/`.
- `bun run build` passes from the checkout root.
- `bun run test` passes from the checkout root.

## E2E Tests

N/A — this slice adds no UI, persistence, or protocol surface. Adapter-boundary
functional tests exercise the production payload constructors.

## Manual / cURL Tests

No HTTP API is added. A reviewer can inspect the focused conformance gate with:

```bash
cargo test -p bridge-core context_inventory -- --nocapture
```

Expected: Claude, Codex, and OpenCode each have complete fail-closed catalog and
turn-presented observations across start, resume, and per-turn phases.
