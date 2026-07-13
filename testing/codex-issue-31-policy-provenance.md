# codex/issue-31-policy-provenance — Test Contract

## Functional Behavior

- A write-capable delegation is eligible only when every claimed owned path is covered by an explicit `write scope:` declaration in the latest durable `user.message`, or by a user-accepted policy approval tied to the same parent turn.
- Path evidence is accepted only when its literal base exists in the real workspace tree or its immediate parent exists for a new file, and its canonical target remains inside the workspace. Historical, negated, fenced/quoted diagnostic, assistant, and delegation-envelope path mentions never create ambient authority.
- A broader claim than the user-approved scope, or a write claim with no trusted scope, returns `require_user_approval` with an owned-path-provenance reason before worker reuse, budgeting, lease, or spawn decisions and creates a resolvable `approval.requested` entry.
- Read-only delegations do not require write-path provenance and retain the existing depth, budget, concurrency, and capability behavior.
- Policy decision records include the normalized trusted paths and their source entry IDs so the approval/spawn boundary is inspectable in the session forest.
- `docs/delegation-policy.md` states explicitly that deterministic policy validates structure and trusted path provenance; it does not judge the semantic quality of an objective.

## Unit Tests

- `write_claim_without_provenance_requires_user_approval` — an otherwise valid write request cannot spawn without trusted scope.
- `write_claim_must_not_be_broader_than_provenance` — evidence for `src/auth/session.rs` does not authorize `src/**`.
- `read_only_request_does_not_require_write_provenance` — a read-only request remains eligible without owned-path evidence.
- `user_path_evidence_uses_latest_explicit_scope_and_workspace_facts` — only an explicit scope in the latest active-branch user turn that resolves against the workspace is accepted.
- `historical_negated_and_diagnostic_paths_do_not_authorize_writes` — old turns and arbitrary prose do not grant scope.
- `new_file_scope_requires_existing_immediate_parent` — a new file is allowed only beneath an existing immediate parent, not merely an existing top-level component.
- `symlinked_scope_cannot_escape_the_workspace` — explicit and approved scopes cannot traverse a repository symlink to an external target.
- Recursive scopes containing nested symlinks, and trusted scopes with non-terminal wildcard components, cannot broaden authority.
- Recursive scope inspection allows workspace-contained links, rejects escaping links, and fails closed at a bounded traversal budget.
- `prior_write_decision_binds_scope_to_its_originating_turn` — a later user turn cannot retroactively broaden an earlier deferred request.
- `policy_approval_is_idempotent_and_stale_branches_cannot_resolve` — repeated identical requests create one card and an abandoned-branch card cannot grant current-branch authority.
- Durable conversation projection and component tests verify resolution folding and removal of the misleading session-wide policy action.
- `assistant_and_request_paths_do_not_create_provenance` — model-authored text and `relevantFiles`/`ownedPaths` fields cannot self-authorize.
- Existing policy normalization, overlap, budget, tier, retry, queue, and persistence tests remain green.

## Integration / Functional Tests

- Parent user turn declaring `write scope: <repository path>` → matching write delegation → `spawn_worker`/`resume_worker` remains possible and the decision entry carries provenance.
- Parent user turn with no explicit write scope → write delegation → resolvable `approval.requested`; no worker session, lease, or spawn-usage row is created before acceptance.
- Accepted policy approval → the still-active same-turn request is re-evaluated with component-safe, workspace-contained approval provenance and may launch; stale-branch, duplicate, session-wide, declined, or cancelled approval never launches.
- Durable UI projection → one terminal card per turn/scope with no session-wide action → native and provider-shaped resolutions fold into that card after reload; accepted launch failure is persisted and surfaced as retryable.
- A typed `Launched` outcome requires successful objective delivery to the worker runtime; missing or rejecting runtimes produce the durable retryable failure path.
- Parent user turn naming a narrow file → delegation claiming a broader directory → `require_user_approval` with an auditable reason.

## Smoke Tests

- `cargo test --manifest-path src-tauri/Cargo.toml policy`
- `npm test`
- `npm run check`

## E2E Tests

- The deterministic policy-coordinator tests are the local E2E-equivalent for this backend safety boundary: durable user entry, workspace resolution, route decision, and forest audit record are exercised together.
- Live provider execution is not required to prove the deny/escalate invariant because the policy gate runs before any provider child starts.

## Manual / cURL Tests

- N/A for cURL — delegation is a local Tauri/SQLite workflow.
- Inspect a recorded `delegation.requested` entry and confirm `reason=owned_path_provenance_required`, with trusted paths/source entries present and no model-authored field listed as a trusted source.
- Confirm `rg -n -i 'refund' . -g '!target' -g '!node_modules' -g '!architecture.excalidraw'` finds no separate refund mechanism; this contract is verified through the requested review-checkpoint workflow.
