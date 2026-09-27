# Full Access Worker Scope - Test Contract

Base: main b05384b.

## Functional Behavior
- Full access (`autoApproveProviderPermissions`) authorizes the write scope a worker proposes. No write-scope card is raised, named or unnamed scope alike. This reverses the earlier rule in `fix-worker-approval-lifecycle.md` that kept delegation approvals human-owned under Full access.
- Each Full access grant is written to the `approval.auto_allowed` ledger, so it shows in Settings > Permissions > Recent auto-approvals. A scope the user already declared with `Write scope:` logs nothing.
- Full access does not make an invalid owned path valid and does not lift hard limits (depth, budgets, retries, concurrency).
- A writable worker that names no owned paths (OpenCode, or the PR reviewer on a harness without read-only) gets a card that says "no path limit". Accepting it launches that worker. It used to fail every time with "already resolved", and the orchestrator retried into a new card each turn.
- An accepted unscoped card authorizes only an unscoped claim in that turn. A named scope is still its own decision. An accepted card whose paths fail grounding grants nothing.
- Browser outward effects and agent prompt changes still ask under Full access.

## Unit Tests
- policy: Full access passes the scope gate but not a hard limit; an unscoped grant covers only an unscoped claim.
- policy_coordinator: accept an unscoped card, then reserve; Full access spawns without a card and audits exactly the grants it made; invalid paths still reject; a grounding failure is not an unscoped grant.

## Frontend Tests
- AgentConversation: an unscoped delegation card renders "No path limit (the worker named none)".
- PermissionsSection: worker write scope is no longer listed under "Always asks".

## Smoke Tests
- bun run build
- bun run test
- git diff --check

## Manual Tests
With Full access on, delegate an OpenCode worker that cannot run read-only and confirm it starts with no card and one Recent auto-approvals row. With User approval on, repeat and confirm one "No path limit" card whose Allow once starts the worker.
