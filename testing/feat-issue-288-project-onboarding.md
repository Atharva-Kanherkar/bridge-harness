# feat/issue-288-project-onboarding — Test Contract

## Functional Behavior

- There is no separate "Add a project" dialog. The welcome composer is the only entry point: the user types what they want and Bridge resolves it.
- Pasting a bare git URL, or an `owner/repo` GitHub shorthand, as the entire welcome message clones it (destination defaults beneath the configurable Bridge-managed project root) and lands directly in the new workspace — no confirmation step, no destination prompt.
- Any other welcome message (including one that merely mentions a URL among other words) is treated as a normal chat message and starts an ordinary session, falling back to the existing pathless scratch-directory session start.
- Choosing a known local folder still goes through the native OS folder picker via the composer's `+` action.
- Failures (invalid URL, clone failure) surface as the existing inline error banner and do not create/register a workspace.
- Existing workspace/session behavior is unchanged once the workspace is registered.

## Unit Tests

- `repoCloneTarget` (`src/workspaceFolder.ts`) recognizes bare git URLs and `owner/repo` shorthand, expands shorthand to a GitHub URL, and rejects anything with surrounding text, whitespace, or a file-path-shaped extension.
- `clone_workspace_repo` (backend) validates its URL and destination, clones with Git, and registers the resulting directory using the existing workspace connection path — unchanged.

## Integration / Functional Tests

- Submitting a bare repo URL from the welcome composer calls `bridgeApi.cloneWorkspaceRepo` and does not send it as a chat turn.
- Submitting an ordinary message from the welcome composer does not call `bridgeApi.cloneWorkspaceRepo`.

## Smoke Tests

- `bun run build` completes successfully.
- `bun run test` completes successfully.

## E2E Tests

N/A — desktop native dialog and authenticated GitHub state are not available in automated browser tests.

## Manual Tests

1. Open the welcome screen, paste a cloneable repository URL as the entire message, and send it.
2. Confirm the cloned project is selected immediately, with no intermediate dialog, and can start a chat.
3. Type an ordinary first message (e.g. a question) and confirm it starts a normal chat instead of attempting a clone.
4. Paste an invalid/unreachable URL and verify the composer shows an error without adding a workspace.
