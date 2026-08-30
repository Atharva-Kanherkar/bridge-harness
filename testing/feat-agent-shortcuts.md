# feat-agent-shortcuts — Test Contract

## Functional Behavior

- A leading `#agent <objective>` directive resolves an enabled, non-orchestrator configured agent and removes the directive token from the worker objective.
- Resolution is case-insensitive and deterministic across configured names, IDs, normalized role labels, and canonical aliases such as `verifier`, `reviewer`, and `implementer`.
- An in-progress leading token such as `#ver` produces deterministic autocomplete candidates populated only from enabled worker-capable agents; selecting the Verification agent inserts `#verifier ` (or that agent's canonical configured token).
- Autocomplete exposes the resolved agent's name, role, and effective dispatch characteristics without making the browser authoritative for role, model, write mode, or owned paths.
- On submission, the host re-resolves the submitted token from persisted agent configuration. Unknown, ambiguous, disabled, orchestrator, and empty-objective directives return actionable, recoverable errors before any provider turn starts.
- A valid direct directive becomes a canonical worker request and uses the existing worker-policy, reservation, concurrency/queueing, approval, lifecycle, evidence, worktree, and completion-gate paths without an orchestrator model turn.
- Verification dispatch remains read-only. Implementation dispatch continues to require established write/path-scope authorization and creates the same completion-plan and independent-verification requirements as an orchestrator-originated implementation request.
- The objective, excluding the `#agent` token, is persisted as the user request together with direct-dispatch provenance, and the resulting worker remains associated with the originating conversation.
- The UI clearly reports launched, queued, and awaiting-approval outcomes and keeps the composer recoverable on rejection.
- Only a directive at the start of the composer is reserved. Existing `$harness` shortcuts, slash commands, `@file` mentions, ordinary `#` prose, and `#` occurring after other text retain current behavior.

## Unit Tests

- `agentMention` parser tests cover complete directives, empty objectives, leading-position enforcement, case normalization, whitespace, in-progress queries, and non-directive `#` prose.
- `agentMention` matching tests cover configured names, IDs, role labels, canonical aliases, deterministic ordering, ambiguity, disabled agents, and orchestrator exclusion.
- UI unit tests cover autocomplete visibility/filtering, deterministic selection/insertion, submit interception, recoverable rejection, and launched/queued/awaiting-approval outcomes.
- Existing `harnessShortcut` tests continue to pass unchanged.
- Rust agent-configuration/API tests cover persisted-config re-resolution, case-insensitive custom names, canonical aliases, ambiguity, unknown/disabled/orchestrator rejection, empty objectives, and the absence of a provider turn on rejection.
- Rust dispatch tests prove that the canonical worker request derives its role, model profile, capability/write policy, and owned paths from host configuration rather than client input.

## Integration / Functional Tests

- A valid verification directive enters the existing reservation path and produces the same launch or queue lifecycle/evidence records as an orchestrator-originated verification worker, with no orchestrator provider turn.
- A valid implementation directive enters existing approval/worktree/completion-gate machinery and preserves independent verification requirements.
- Reservation outcomes are mapped back to the originating conversation and rendered as launched, queued, or awaiting approval.
- Invalid directives fail before provider startup and do not mutate worker lifecycle state.
- Frontend and Rust test suites remain green together.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds.
- Focused frontend parser/UI tests and focused Rust resolution/dispatch tests succeed during implementation.

## E2E Tests

- Automated component/API integration coverage exercises `#verifier verify X` and `#implementer implement X` through submission, host re-resolution, and reservation-outcome handling.
- Full desktop provider execution is not required because it would consume external provider capacity; existing fake-provider/reservation fixtures must prove that no orchestrator turn is started and that the normal worker path is selected.

## Manual / cURL Tests

- In the composer, type `#ver`; verify a deterministic Verification suggestion, select it, and confirm the inserted token has a trailing space.
- Submit `#verifier verify the current implementation`; verify the worker is launched or queued directly and the directive itself is not sent to the orchestrator.
- Submit `#implementer add retry handling`; verify normal implementation approval/path-scope and completion-gate UI is used.
- Try an unknown token, ambiguous alias, disabled agent, `#orchestrator`, and a directive with no objective; verify recoverable local feedback and no provider turn.
- Submit ordinary prose containing `#`, a `$harness` shortcut, a slash command, and an `@file` mention; verify their existing behavior is unchanged.
- No cURL test is applicable: this is a Tauri invoke/protocol flow rather than an HTTP endpoint.
