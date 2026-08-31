# feat/issue-288-project-onboarding — Test Contract

## Functional Behavior

- The folder-first welcome action remains the direct way to create a project from a known local folder.
- A user can choose an **Add project** action and select **Clone from URL** or **Search GitHub** without leaving Bridge.
- Cloning uses a user-editable destination, defaulted beneath the configurable Bridge-managed project root, registers the cloned folder as a workspace, and returns the updated `BridgeState`.
- GitHub search returns only repositories available to the authenticated `gh` user. Selecting a result starts the same clone-and-register path.
- Failures (invalid URLs, clone failures, no GitHub results, missing `gh` authentication) are reported explicitly and do not create/register a workspace.
- Existing workspace/session behavior is unchanged once the workspace is registered.

## Unit Tests

- `clone_workspace_repo` validates its URL and destination, clones with Git, and registers the resulting directory using the existing workspace connection path.
- `search_github_repos` invokes `gh repo list`/search safely, parses the expected repository fields, and distinguishes an empty result from command failure.
- Frontend API wrappers invoke the matching native commands with their request payloads.
- The onboarding dialog exposes clone and GitHub search options and sends the selected result through the clone path.

## Integration / Functional Tests

- A temporary local Git repository cloned by the backend becomes a registered workspace in the returned state.
- A selected GitHub result populates the clone URL and follows the same registration flow as a pasted URL.

## Smoke Tests

- `bun run build` completes successfully.
- `bun run test` completes successfully.

## E2E Tests

N/A — desktop native dialog and authenticated GitHub state are not available in automated browser tests.

## Manual Tests

1. Open the welcome screen and choose **Add project → Clone from URL**.
2. Paste a cloneable repository URL, verify the destination is editable, then clone it.
3. Confirm the cloned project is selected and can start a chat.
4. Choose **Add project → Search GitHub**, search for a visible repository, select it, and verify it reaches the same clone confirmation.
5. Enter an invalid URL and verify the dialog shows an error without adding a workspace.
