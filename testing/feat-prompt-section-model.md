# feat/prompt-section-model — Test Contract

## Functional Behavior
- Orchestrator defaults expose exactly `bridge_role` and `delegation_protocol`; worker defaults expose exactly `worker_contract`; direct sessions expose no Bridge-stable sections.
- With no overrides, complete compiled prompt bytes remain identical to the current live constructors for orchestrators, all five worker roles, and direct sessions, including restoration context variants.
- `RENDERING_NOTE` remains embedded in the orchestrator `bridge_role` and remains absent from the live worker and direct-session stacks. This PR does not introduce a worker rendering-note behavior change.
- An override replaces only the selected target and section. Worker-role overrides do not affect other worker roles.
- Deleting a section is persisted as an explicit state and omits that section from the compiled stable envelope.
- Resetting a section removes its active override/deletion and resolves the built-in default again.
- Every override, delete, reset, and restore appends an immutable revision. Restoring a revision copies its state into a new revision rather than rewinding history.
- Invalid target/section combinations are rejected. Direct sessions cannot acquire orchestrator or worker stable policy.
- Stable overrides containing secrets or session-capability material are rejected before persistence.
- Required orchestrator vocabulary is defined once in `REQUIRED_MARKERS`; lint reports missing markers without blocking user edits, including a typed-delegation warning when `bridge-delegate` is removed.
- Existing configured project rules and variable sections retain their current target-specific composition and ordering.

## Unit Tests
- `target_inventories_match_the_live_prompt_shapes` — validates exact stable section IDs for orchestrator, every worker role, and direct session.
- `required_marker_lint_agrees_with_the_default_briefing` — the default orchestrator stack is clean and removing `bridge-delegate` produces a typed-delegation warning.
- `override_delete_reset_and_restore_round_trip` — validates active tri-state persistence, append-only revisions, and restore-as-new-revision behavior.
- `worker_overrides_are_role_specific` — a worker-role override does not affect another role.
- `invalid_sections_and_unsafe_overrides_are_rejected` — rejects cross-target section IDs, secrets, and session-capability text.
- `agent_reset_all_does_not_bypass_prompt_revision_history` — existing settings reset cannot silently erase Prompt Studio state.
- `default_target_stacks_match_legacy_live_bytes` — compares complete instructions for orchestrator, all worker roles, restoration variants, and direct sessions.
- `worker_live_stack_preserves_rendering_note_omission` — pins the explicit no-change decision for worker rendering instructions.
- Existing migration parity tests continue to pass with the prompt revision schema.

## Integration / Functional Tests
- Compile a prompt from a persisted override, deletion, reset, and restored revision through the same resolver used by `live_turn`; verify only the applicable stable section changes.
- Open both a fresh database and an upgraded legacy fixture; verify schema signatures and migration histories match.

## Smoke Tests
- `cargo test -p bridge-core prompt` passes.
- `cargo test -p bridge-core` passes.
- `bun run check`, `bun run build`, and `bun run test` pass from the checkout root.

## E2E Tests
N/A — this child issue intentionally adds no protocol or UI surface; those belong to #241 and #242.

## Manual / cURL Tests
N/A — no network API is introduced. Review the generated stable envelopes in Rust tests to confirm exact section IDs, direct-session isolation, and worker rendering-note omission.
