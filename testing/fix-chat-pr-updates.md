# fix/chat-pr-updates Test Contract

## Functional Behavior
- A completed gh pr create with a PR URL attaches to its chat when repository, branch and SHA match the workspace root or one of its linked worktrees.
- Shell-wrapped gh commands are detected. Prose, echo commands, failed events and unrelated repositories or heads do not attach.
- Refreshing the GitHub pane preserves attached PR watchers and their state/check baselines, including pending PRs listed by the pane.
- Failed checks reads preserve last known checks as stale and keep the watcher due for retry; they never replace a successful snapshot with empty checks.
- Refresh requests arriving during an in-flight read retain their forced-refresh intent. Focus retries discovery even when no cards are loaded.

## Unit Tests
- Rust regression tests cover linked checkout verification, shell wrappers, watcher preservation, and failed check observations.
- Vitest hook tests cover queued forced refresh and focus after an empty read.

## Integration / Functional Tests
- Use temporary Git repositories with linked worktrees to validate actual branch/SHA verification.
- Run bun run build and bun run test successfully before opening the PR.

## Smoke Tests
- Targeted Rust and frontend regression suites pass.

## E2E Tests
- N/A for automated desktop E2E: no desktop test driver configured for this change.

## Manual / cURL Tests
- Reviewer: create a PR from a linked worktree in a Bridge chat; confirm its card appears automatically, open the GitHub pane while checks run, and confirm the card follows completion.
- Reviewer: interrupt GitHub connectivity, retry the card, and confirm last known checks remain stale until connectivity returns.
- Live desktop verification is recorded separately when available; these reviewer steps are not claimed as executed by unit tests.
