# codex/issue-31-policy-provenance — Test Contract

## Functional Behavior

- A write-capable delegation is eligible only when every claimed owned path is covered by repository-relative path evidence extracted from durable `user.message` entries on the parent session's active branch.
- Path evidence is accepted only when its literal base exists in the real workspace tree, or its nearest parent exists for a new file. Assistant prose and delegation-envelope fields never create trusted scope.
- A broader claim than the user-derived scope, or a write claim with no trusted scope, returns `require_user_approval` with an owned-path-provenance reason before worker reuse, budgeting, lease, or spawn decisions.
- Read-only delegations do not require write-path provenance and retain the existing depth, budget, concurrency, and capability behavior.
- Policy decision records include the normalized trusted paths and their source entry IDs so the approval/spawn boundary is inspectable in the session forest.
- `docs/delegation-policy.md` states explicitly that deterministic policy validates structure and trusted path provenance; it does not judge the semantic quality of an objective.

## Unit Tests

- `write_claim_without_provenance_requires_user_approval` — an otherwise valid write request cannot spawn without trusted scope.
- `write_claim_must_not_be_broader_than_provenance` — evidence for `src/auth/session.rs` does not authorize `src/**`.
- `read_only_request_does_not_require_write_provenance` — a read-only request remains eligible without owned-path evidence.
- `user_path_evidence_uses_active_branch_and_workspace_facts` — only path literals from active `user.message` entries that resolve against the workspace are accepted.
- `assistant_and_request_paths_do_not_create_provenance` — model-authored text and `relevantFiles`/`ownedPaths` fields cannot self-authorize.
- Existing policy normalization, overlap, budget, tier, retry, queue, and persistence tests remain green.

## Integration / Functional Tests

- Parent user turn naming a repository path → matching write delegation → `spawn_worker`/`resume_worker` remains possible and the decision entry carries provenance.
- Parent user turn with no repository path → write delegation → `require_user_approval`; no worker session, lease, or spawn-usage row is created.
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
