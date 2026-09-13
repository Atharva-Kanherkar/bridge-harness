# Worker Approval Lifecycle - Test Contract

Base: main fdc5968aa17e672e48f839731707e36e41aaefa5.

## Functional Behavior
- A delegation's policy rejection or pending approval cannot be hidden by a cross-harness queue decision. A new child is not a migration of the parent's active runtime.
- Accepted write scope produces a child ID, an observable durable queue ID, or an explicit terminal failure. Resolved approval cards never reopen.
- Repeated decisions cannot start the same approved request twice. Pending and failed outcomes remain distinct.
- Queued work progresses independently of the parent's provider, and terminal queue expiry/failure is visible to its parent.
- Explicit Write scope uses the originating user message, including after an earlier decision had no usable provenance. It never borrows a later turn's authorization.
- Approved and explicit paths can name not-yet-created nested directories. Both reject out-of-workspace paths, dangling/escaping symlinks, and invalid patterns.
- Deterministic budget/retry rejection precedes prompting for a write scope.
- Full access auto-resolves only eligible provider permission requests, using the correct durable event identity even in a multi-event frame. Host delegation and prompt-change approvals remain human-owned.
- Delegation instructions document the supported networkAccess and writableOutputPaths fields and their defaults. Read-only mode is not silently widened to allow writes or unrestricted shell.

## Unit Tests
- policy: exhausted budgets/retries reject before approval; resolved approvals are not reused as pending.
- policy_coordinator: nested new paths, symlink containment, origin-message fallback, and no later-turn scope borrowing.
- live_turn: multi-event provider approvals, cross-harness reservations, approved scope retry, durable queue identities.
- worker_pool: cross-harness queue progress, terminal expiry reporting, stable queue identity.
- orchestrator/delegation: prompt examples parse and capability defaults are explicit.

## Integration / Functional Tests
- In-memory SQLite forest plus real temporary workspace: request scope, accept, re-evaluate, reserve exactly once without a second approval card.
- Queue a capacity-blocked delegation and verify its durable ID/status; release capacity and claim it without requiring a parent turn boundary.
- Existing provider interaction, provenance, sandbox, worker lifecycle and frontend projection tests remain green.

## Smoke Tests
- bun run build
- bun run test
- git diff --check

## E2E Tests
Live provider execution requires an installed running build and account access. Do not launch workers in this session: the user explicitly prohibited them. Use deterministic host/adapter fixtures instead; report live desktop coverage as not run.

## Manual Tests
In a rebuilt app, delegate an isolated edit into a new nested directory, approve once, and verify one child or a queue ID. Repeat the click and confirm no duplicate child. With Full access, verify provider permissions resolve without actionable Allow once/Allow always cards. With networkAccess false, verify network remains unavailable; request true explicitly for online research. These live checks are not authorized during this session.

## Issue Coverage
Fix #636's repeated approval, null-child queued acknowledgement, and stalled launch. Audit adjacent worker capability and approval defects. #425 (new session modes) and #440 (recurring byte-bound grants) remain separate feature proposals. No claim to fix unrelated worker-cache, TUI, or connector work.
