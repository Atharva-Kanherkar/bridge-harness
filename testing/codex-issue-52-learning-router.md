# Issue 52 — Cost-and-Quality Learning Router Test Contract

## Functional Behavior

- The router evaluates every available harness/model candidate against normalized tool, permission, platform, quota, context, risk, capability, and user-preference constraints.
- Every recommendation records the complete candidate set, deterministic exclusions, predicted pass probability, latency, normalized quota cost, retry risk, selected route, explanation, router mode, and the eventual actual outcome.
- Shadow mode records what the learning router would choose without changing the existing deterministic policy decision. Autonomous mode may select an eligible candidate, but can never weaken deterministic permission, topology, write-safety, or budget ceilings.
- Bounded escalation moves only to a more capable eligible route after a failed outcome and never retries the same route indefinitely.
- Workspace preferences can pin or exclude harnesses and models. Invalid, unavailable, excluded, over-budget, or policy-ineligible pins fail closed with an auditable explanation.
- Historical outcomes update deterministic per-candidate quality estimates without silently changing safety policy. Sparse or missing history falls back to explicit conservative priors.
- Manual route overrides are recorded separately from router recommendations and remain attributable in outcome data.

## Unit Tests

- Candidate evaluation returns stable, reason-coded exclusions for unavailable capabilities, tools, platform, permissions, quota, context, risk, and user exclusions.
- Candidate scoring prefers the lowest normalized expected cost that satisfies the configured quality floor; ties resolve deterministically.
- Sparse-history estimates use documented priors, and recorded outcomes update pass probability, latency, normalized cost, and retry risk.
- A pinned eligible candidate wins; an ineligible pin cannot bypass a ceiling.
- Shadow mode preserves the baseline route while recording the alternative recommendation.
- Autonomous mode selects only from eligible candidates and respects the maximum capability-unit budget.
- Escalation is bounded and strictly increases capability tier or produces a terminal no-route result.
- Router decisions and outcomes serialize and deserialize without losing candidate or exclusion details.

## Integration / Functional Tests

- SQLite migrations create router preference, decision, candidate, and outcome persistence idempotently.
- A routed worker request records its router decision before queue or launch and attaches the eventual worker result as actual outcome data.
- Existing policy replay remains deterministic and router replay can compare shadow recommendations with recorded actual routes without starting providers or mutating the database.
- Existing delegation, worker queue, lifecycle, usage, approvals, and write-scope behavior remain unchanged when the learning router is disabled or in shadow mode.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds, including the complete Rust and frontend suites.
- `bun run check` succeeds.
- `git diff --check` reports no whitespace errors.

## E2E Tests

- N/A for provider execution: routing behavior is exercised against deterministic fixtures so tests do not consume subscription quota or require provider credentials.
- A held-out fixture set reports recommendation coverage, manual-selection rate, predicted versus actual quality, latency, normalized cost, retry rate, and policy violations. The fixture proves the reporting path and a manual-selection rate below 5%; it does not claim real-world model superiority until production outcome data exists.

## Manual / CLI Tests

- Run router replay against a temporary database fixture and confirm it emits candidates, exclusions, recommendation, baseline route, mode, and outcome comparison as JSON.
- Inspect an ineligible pinned route and confirm the explanation identifies the deterministic constraint rather than silently falling back.
- Inspect a shadow decision and confirm the executed route remains the baseline deterministic route.
