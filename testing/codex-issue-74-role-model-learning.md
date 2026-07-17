# codex/issue-74-role-model-learning — Test Contract

## Functional Behavior

- A fresh database reports model setup as incomplete and offers one **Use recommended defaults** action before the normal Bridge home experience.
- Recommended profiles are derived only from available adapter catalog entries and `defaultForTier`; no frontend profile contains a hardcoded provider model ID.
- Setup persists version 1 profiles for standard orchestrator, premium orchestrator, planner, implementer, verifier, reviewer, research, documentation, and evaluator purposes.
- Each stored profile has a stable profile ID, and the persisted Standard orchestrator provider/model/tier/effort—not adapter enumeration order—drives new orchestrator and direct-chat defaults.
- Reviewer and evaluator purposes retain the canonical `verification` runtime role; the runtime role vocabulary does not grow validator/judge/QA aliases.
- Advanced setup can select only catalog-supported provider/model pairs, reasoning effort, fallback profile, pin/learning behavior, and optional budget/latency preference.
- Saving or resetting setup creates a new immutable profile version and leaves prior versions queryable.
- If a configured model is unavailable, profile resolution uses the live adapter catalog and follows the configured fallback (or a catalog default) without rewriting profile history.
- The usage ledger durably stores normalized monetary cost when a provider reports it and preserves unknown cost as `NULL`.
- Every routed run binds its decision and outcome to trace/workspace/session/turn, task fingerprint, repository revision, profile version, policy version, catalog snapshot, eligible and excluded candidates, selected and actual provider/model/effort, selection reason, override state, latency, provider-reported cost, retry/edit/intervention signals, acceptance, and source confidence. Missing metrics remain `NULL`/unknown.
- User profile pins, router pins/exclusions, permission/sandbox/tool constraints, provider availability, and request budgets are deterministic eligibility gates. Learning may rank only the candidates that pass those gates.
- **Run learning now** invokes the same durable learning runner used by every trigger and returns queued/running/completed/failed/cancelled or no-op state plus a cost/quality/policy-diff report.
- The runner acquires an expiring durable lease, freezes an evidence high-water mark, uses an idempotency key derived from job ID + evidence boundary + base policy version, and allows at most one active run for that snapshot. Duplicate, expired, and unauthorized triggers are visible, auditable no-ops.
- One durable job-wide lease also prevents different snapshots from running concurrently. Expired lease recovery uses compare-and-swap, and small evidence batches accumulate behind a cursor that advances only after evaluation consumes them.
- Insufficient evidence and exhausted learning budget are successful, auditable no-ops; they never fabricate an improved policy.
- The learning pipeline runs deterministic evaluations first, records bounded model-evaluator identity where needed, aggregates evidence by task fingerprint/profile/provider/model/effort, and creates an immutable candidate only after minimum sample/confidence thresholds pass.
- Historical replay compares realized quality, cost per successful task, latency, retry rate, and intervention rate across policy versions. Cost comparison is unknown unless every included run reports cost.
- Manual mode creates an immutable recommendation only. Ask mode requires a separate approval command before promotion. Automatic mode promotes only after held-out replay and guardrails pass.
- Promotion is an atomic transaction that archives the predecessor, activates one version, records the explanation, and preserves a reversible predecessor chain. Rollback atomically creates/activates a new version derived from a prior policy rather than rewriting history.
- Automatic promotion enters a canary state; subsequent evidence is checked for regression and automatically rolls back before full activation when quality, cost, latency, retry, or intervention guardrails regress.
- Automatic mode cannot stack a second canary while the first is gathering evidence, and deterministic hash bucketing keeps canary traffic near the configured 20% share.
- Cancellation requested before promotion marks the run cancelled. No trigger adapter or model evaluator can directly promote policy.
- Each learning run records actual evaluator spend/tokens and an explicit execution state. The current evaluator is deferred rather than executed, so those counters remain zero and a zero ceiling prevents queuing; positive configurable ceilings are reserved for the future bounded evaluator executor rather than presented as active enforcement.
- In-app scheduling persists `nextRunAt`, performs at most one startup/due catch-up, advances the next due time from the current run, and never emits one run per missed interval.
- Codex and Claude scheduling are represented as optional trigger adapters over the same runner. The UI exposes truthful, copyable setup guidance; both firing for the same snapshot produces one run and an auditable duplicate no-op.
- Only one external trigger provider is enabled by default. Trigger metadata stores credential references and an authentication digest only; raw bearer tokens are rejected and never enter session history. Disabled, removed, expired, or incorrectly authenticated registrations cannot start learning.
- Codex setup ships a tested scheduled-task prompt/skill around the narrow local command. Claude local scheduling is preferred; cloud Routine support is explicitly experimental and is only an authenticated wake-up when a Bridge-owned endpoint exists.
- Direct-chat model switching continues to work independently of role profiles.

## Unit Tests

- `model_profiles::recommended_profiles_use_catalog_defaults` — all defaults come from available catalog entries marked `default_for_tier`.
- `model_profiles::review_purposes_map_to_verification` — reviewer/evaluator purposes use the canonical verification role.
- `model_profiles::save_profiles_versions_immutably` — editing/resetting increments versions and preserves earlier rows.
- `model_profiles::resolve_profile_falls_back_when_model_disappears` — live catalog resolution never returns an unavailable model.
- `orchestrator_start_uses_the_persisted_standard_profile` — new orchestrators resolve the stored Standard profile rather than using adapter order or a hardcoded tier.
- `model_profiles::rejects_unsupported_or_recursive_profiles` — invalid catalog selections and fallback cycles fail validation.
- `learning_job::duplicate_triggers_share_one_snapshot_run` — simultaneous/manual/external trigger attempts create at most one run.
- `learning_job::insufficient_evidence_is_auditable_noop` — cold start completes without a candidate policy.
- `learning_job::recommendation_mode_never_promotes` — candidate generation cannot alter the active policy.
- `learning_job::budget_and_secret_guards_are_deterministic` — spend/token ceilings skip work and raw trigger secrets are rejected.
- `learning_job::lease_expiry_and_trigger_auth_are_auditable` — active leases deduplicate, expired leases recover safely, and expired/disabled/unauthorized registrations record no-op events.
- `learning_job::one_job_lease_blocks_a_competing_newer_snapshot` — the partial unique lease prevents concurrent runs even when their frozen boundaries differ.
- `learning_job::subsequent_runs_require_an_incremental_evidence_batch` — sub-threshold arrivals accumulate and the cursor advances only after a complete batch is evaluated.
- `learning_job::ask_requires_approval_and_promotion_rollback_are_versioned` — Ask never promotes inline; approval and rollback preserve immutable predecessor history.
- `learning_job::automatic_mode_never_stacks_canary_promotions` — Automatic cannot create a second candidate/canary while one is active.
- `learning_job::automatic_canary_rolls_back_on_regression` — a guardrail regression reverses the canary atomically.
- `learning_job::automatic_canary_holds_when_required_cost_is_unknown` — an unknown provider cost holds the canary for more evidence instead of creating a promote/rollback treadmill.
- `learning_job::failed_run_abandons_its_linked_candidate_atomically` — failure cleanup transactionally abandons a linked candidate rather than leaving an orphan.
- `learning_job::cost_per_success_requires_complete_provider_costs` — missing cost stays unknown and complete cost divides total spend by successful outcomes.
- `learning_job::due_schedule_catches_up_once` — missed intervals advance from now and create one catch-up run.
- `learning_job::replay_fixtures_cover_cost_regression_and_unavailable_models` plus the recommendation/no-op/canary tests — quality improvement, cost regression, insufficient evidence, unavailable models, and rollback are deterministic fixtures.
- `learning_router::profile_quality_preference_changes_ranking_without_widening_eligibility` — advanced cost/latency preferences affect ranking but cannot revive excluded candidates.
- `UsageReport` parsing tests — reported cost is retained; absent provider cost remains unknown.
- Frontend profile helpers — catalog choices exclude unavailable/unsupported models and produce stable profile labels.
- Frontend profile helpers and settings integration — unavailable adapters do not trap the user in setup, and an unrelated settings save does not create a new profile version.

## Integration / Functional Tests

- Migration 15 upgrades an existing version-14 database, preserves existing router data, adds every minimum issue record/field (stable profile IDs, trace-bound decisions, immutable policies, complete outcomes/evaluations, job/trigger/run leases, incremental evidence cursor, built-in/external trigger metadata, promotion audit, usage cost), and is idempotent/transactional.
- Tauri model-setup commands round-trip recommended and customized profiles with immutable versioning.
- Tauri learning commands use the single runner for manual, in-app, Codex, and Claude trigger kinds; expose approval, rollback, cancellation, and persisted reports without giving external adapters promotion authority.
- The existing learning router resolves a role profile through the live adapter registry before delegating, while deterministic policy remains authoritative.
- Routed execution persists the live catalog snapshot and the actual post-resolution provider/model/effort, then builds typed outcome/evaluation records from normalized events without concatenating raw transcripts.
- Routing cost accounting keeps capability-normalized quota cost separate from provider-reported micro-USD, and evidence-recording failure cannot prevent a worker launch.
- Learning runs use a dedicated SQLite connection and a bounded replay window so the global application connection is not held while history is loaded and evaluated.
- The learning runner extends policy replay with realized outcomes and held-out guardrails before any promotion.
- Frontend API mock and native command shapes agree for setup status, profile save/reset, learning status/run/cancel, and schedule update.
- Setup and learning UI tests cover recommended defaults, advanced disclosure, manual run state, failed/duplicate jobs, Ask approval, Automatic/canary/rollback state, cost confidence, and scheduler limitation copy.
- Interactive jsdom journeys click the recommended setup action, disclose advanced catalog-only profiles, and run manual learning through the settings dialog to an explicit no-op report.

## Smoke Tests

- `bun run build` passes.
- `bun run check` passes.
- `bun run test` passes (Vitest, sidecar tests, and Rust tests).
- `git diff --check` passes.
- A fresh in-memory database migrates through version 15 and produces catalog-backed recommended profiles.
- The narrow `bridge learning run` command validates registration/authentication and leaves active desktop sessions untouched.

## E2E Tests

- Component-level journey: incomplete setup → choose recommended defaults → setup becomes complete → normal Bridge home renders.
- Component-level journey: open advanced setup → edit a supported profile → save → reopen and observe the new version.
- Component-level journey: open learning settings → run now → observe a completed/no-op report without active-policy promotion.
- Component-level journey: switch to Ask → approve a replayed candidate → observe one new active version → roll back and observe another immutable version.
- Component-level journey: Automatic mode displays canary/guardrail state and a failed/duplicate external trigger remains an auditable no-op.
- Native desktop scheduler timing is covered by deterministic Rust integration tests; no OS cron or provider cloud E2E is applicable because those integrations are optional, user-managed trigger adapters.

## Manual / cURL Tests

- Start `bun run dev` and verify the mock first-run wizard completes with one click, advanced fields contain only installed adapter catalog models, and direct chat still opens afterward.
- In a workspace, open learning settings, run learning, and verify the report clearly identifies recommendation/no-op state, evidence boundary, cost/quality deltas, and policy base/candidate versions.
- Verify Codex and Claude setup sections describe their real availability limits and copy a narrow `bridge learning run --trigger …` instruction rather than claiming Bridge created a provider schedule.
- No cURL test is applicable: the feature is exposed through typed Tauri commands/local trigger invocation and does not claim the not-yet-existing Bridge-owned Routine endpoint.
