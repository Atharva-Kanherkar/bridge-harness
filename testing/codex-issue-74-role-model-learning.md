# codex/issue-74-role-model-learning — Test Contract

## Functional Behavior

- A fresh database reports model setup as incomplete and offers one **Use recommended defaults** action before the normal Bridge home experience.
- Recommended profiles are derived only from available adapter catalog entries and `defaultForTier`; no frontend profile contains a hardcoded provider model ID.
- Setup persists version 1 profiles for standard orchestrator, premium orchestrator, planner, implementer, verifier, reviewer, research, documentation, and evaluator purposes.
- Reviewer and evaluator purposes retain the canonical `verification` runtime role; the runtime role vocabulary does not grow validator/judge/QA aliases.
- Advanced setup can select only catalog-supported provider/model pairs, reasoning effort, fallback profile, pin/learning behavior, and optional budget/latency preference.
- Saving or resetting setup creates a new immutable profile version and leaves prior versions queryable.
- If a configured model is unavailable, profile resolution uses the live adapter catalog and follows the configured fallback (or a catalog default) without rewriting profile history.
- The usage ledger durably stores normalized monetary cost when a provider reports it and preserves unknown cost as `NULL`.
- **Run learning now** invokes the same durable learning runner used by every trigger and returns queued/running/completed/failed/cancelled or no-op state plus a cost/quality/policy-diff report.
- The runner freezes an evidence high-water mark, uses an idempotency key derived from job ID + evidence boundary + base policy version, and allows at most one active run for that snapshot.
- Insufficient evidence and exhausted learning budget are successful, auditable no-ops; they never fabricate an improved policy.
- The initial learning release creates an immutable recommendation-only candidate policy. It cannot mutate permissions, sandboxing, provider availability, user pins/exclusions, or the active policy.
- Cancellation requested before promotion marks the run cancelled. Manual/recommendation mode never promotes a policy.
- In-app scheduling persists `nextRunAt`, performs at most one startup/due catch-up, advances the next due time from the current run, and never emits one run per missed interval.
- Codex and Claude scheduling are represented as optional trigger adapters over the same runner. The UI exposes truthful, copyable setup guidance; both firing for the same snapshot produces one run and an auditable duplicate no-op.
- Trigger metadata stores credential references only; raw bearer tokens are rejected.
- Direct-chat model switching continues to work independently of role profiles.

## Unit Tests

- `model_profiles::recommended_profiles_use_catalog_defaults` — all defaults come from available catalog entries marked `default_for_tier`.
- `model_profiles::review_purposes_map_to_verification` — reviewer/evaluator purposes use the canonical verification role.
- `model_profiles::save_profiles_versions_immutably` — editing/resetting increments versions and preserves earlier rows.
- `model_profiles::resolve_profile_falls_back_when_model_disappears` — live catalog resolution never returns an unavailable model.
- `model_profiles::rejects_unsupported_or_recursive_profiles` — invalid catalog selections and fallback cycles fail validation.
- `learning_job::duplicate_triggers_share_one_snapshot_run` — simultaneous/manual/external trigger attempts create at most one run.
- `learning_job::insufficient_evidence_is_auditable_noop` — cold start completes without a candidate policy.
- `learning_job::recommendation_mode_never_promotes` — candidate generation cannot alter the active policy.
- `learning_job::budget_and_secret_guards_are_deterministic` — spend ceilings skip work and raw trigger secrets are rejected.
- `learning_job::due_schedule_catches_up_once` — missed intervals advance from now and create one catch-up run.
- `UsageReport` parsing tests — reported cost is retained; absent provider cost remains unknown.
- Frontend profile helpers — catalog choices exclude unavailable/unsupported models and produce stable profile labels.

## Integration / Functional Tests

- Migration 15 upgrades an existing version-14 database, preserves existing router data, adds profile/policy/job/trigger/run/evaluation records and usage cost columns, and is idempotent/transactional.
- Tauri model-setup commands round-trip recommended and customized profiles with immutable versioning.
- Tauri learning commands use the single runner for manual, in-app, Codex, and Claude trigger kinds and return the persisted report.
- The existing learning router resolves a role profile through the live adapter registry before delegating, while deterministic policy remains authoritative.
- Frontend API mock and native command shapes agree for setup status, profile save/reset, learning status/run/cancel, and schedule update.
- Setup and learning UI tests cover recommended defaults, advanced disclosure, manual run state, duplicate/no-op reports, and scheduler limitation copy.

## Smoke Tests

- `bun run build` passes.
- `bun run check` passes.
- `bun run test` passes (Vitest, sidecar tests, and Rust tests).
- `git diff --check` passes.
- A fresh in-memory database migrates through version 15 and produces catalog-backed recommended profiles.

## E2E Tests

- Component-level journey: incomplete setup → choose recommended defaults → setup becomes complete → normal Bridge home renders.
- Component-level journey: open advanced setup → edit a supported profile → save → reopen and observe the new version.
- Component-level journey: open learning settings → run now → observe a completed/no-op report without active-policy promotion.
- Native desktop scheduler timing is covered by deterministic Rust integration tests; no OS cron or provider cloud E2E is applicable because those integrations are optional, user-managed trigger adapters.

## Manual / cURL Tests

- Start `bun run dev` and verify the mock first-run wizard completes with one click, advanced fields contain only installed adapter catalog models, and direct chat still opens afterward.
- In a workspace, open learning settings, run learning, and verify the report clearly identifies recommendation/no-op state, evidence boundary, cost/quality deltas, and policy base/candidate versions.
- Verify Codex and Claude setup sections describe their real availability limits and copy a narrow `bridge learning run --trigger …` instruction rather than claiming Bridge created a provider schedule.
- No cURL test is applicable: the feature is exposed through typed Tauri commands/local trigger invocation and does not add a network endpoint.
