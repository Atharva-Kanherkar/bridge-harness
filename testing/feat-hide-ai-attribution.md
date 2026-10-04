# feat/hide-ai-attribution — Test Contract

## Functional Behavior
- Gitplace header shows a "Hide AI attribution" toggle. ON means models must never add Co-authored-by, Generated-by, or harness mentions anywhere.
- When ON, every prompt Bridge sends to a model starts with a strict no-attribution rule as the first content, so it cannot drift.
- When OFF (default), prompts are unchanged from today.
- Scope when ON covers all prompts: orchestrator briefing, all worker contracts (research, implementation, verification, planning, documentation), direct sessions, and the GitHub PR reviewer objective.
- The toggle persists across restarts (stored in Bridge config, not in-memory only).
- No em dashes in user-visible copy.

## Unit Tests
- `attribution_settings_defaults_to_visible` — fresh DB loads `hideAiAttribution: false`.
- `attribution_settings_round_trips` — save ON loads ON; save OFF loads OFF; blank trims.
- `attribution_rule_is_first_in_orchestrator` — orchestrator resolved stack section 0 id is attribution-hiding with strict text when ON.
- `attribution_rule_is_first_in_workers` — each worker role resolved stack starts with the rule when ON, absent when OFF.
- `attribution_rule_is_first_in_direct_session` — direct session stack starts with rule when ON.
- `reviewer_objective_carries_hiding_rule_when_on` — reviewer objective starts with rule when ON, unchanged when OFF.
- `attribution_settings_hook_defaults_off` — frontend hook reads OFF by default and toggles.
- `gitplace_toggle_renders_and_persists` — Gitplace header toggle calls save and reflects stored value.

## Integration / Functional Tests
- `prompt_sections::resolve` with attribution ON returns the hiding section first for orchestrator, worker:implementation, and direct_session; with OFF returns today's shapes.
- `GET prompt stack` preview for orchestrator with ON shows hiding rule first.
- `reviewerSettings + attributionSettings` save then `objective(7)` starts with hiding rule when ON.
- Frontend: toggle ON then reload Gitplace keeps ON; toggle OFF keeps OFF.

## Smoke Tests
- `bun run build` passes (tsc + vite).
- `bun run test` frontend suite passes for new files.
- `cargo test -p bridge-core attribution` passes.
- Gitplace opens with no chat and shows the toggle next to the repo picker.

## E2E Tests
- N/A — no browser automation for this change; manual verification covers it.

## Manual / cURL Tests
```bash
# Build and unit checks
bun run build
bun run test -- attribution gitplace
cargo test -p bridge-core attribution_settings
cargo test -p bridge-core prompt_sections
```
Expected: all green. Gitplace header shows "Hide AI attribution" switch. Turning it ON adds the strict rule as the first prompt section in Prompt Studio for orchestrator, workers, and direct sessions.
