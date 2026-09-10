# fix/storage-orphan-deletion — Test Contract

## Functional Behavior
- Explicitly confirmed Delete removes an inactive Bridge checkout whose Git registration is missing; safe Reclaim and Sweep retain it.
- Filesystem fallback is restricted to real checkout slots in the Bridge namespace. Never remove a namespace/grouping directory, a symlink target, a standalone repository, an external checkout, a live session checkout, or pending worker output.
- Registered dirty checkouts still support confirmed Delete. Git errors other than missing registration remain failures.
- Successful deletion removes the inventory row, updates usage, and records reclaimed bytes and an audit event.
- Reconciliation drops missing external checkouts from inventory without deleting any external files; repeated reconciliation is idempotent.
- Repository counts sum to the total inventory count, while retention budgets count only Bridge-owned checkouts.
- The native reclaim command accepts the boolean force parameter declared by the protocol and sent by the frontend. The existing command-signature contract test must pass.
- Storage explains retention targets, explicit deletion, external counts and unmeasured sizes accurately.

## Unit Tests
- worktree_registry: orphan deletion, safe retention, dirty deletion, path and ownership guards, live/pending-output guards, external reconciliation and mixed usage/budget regression tests.
- StoragePage: unreadable checkout confirmation, cancellation, force request, removal and updated totals; external/unmeasured context.

## Integration / Functional Tests
- Rust tests use real disposable Git repositories and SQLite; remove fixture Git registration to reproduce the reported fatal error.
- Run bun run build and bun run test before opening the PR.

## Smoke Tests
- Storage component renders totals, repository chart and checkout actions successfully.

## E2E Tests
- N/A — native desktop automation is not configured for this fix; Git/SQLite integration and component interaction tests cover the affected boundaries.

## Manual / cURL Tests
- Reproduce the reported missing-registration state in disposable fixtures only. Do not delete the user's actual checkouts during development.
