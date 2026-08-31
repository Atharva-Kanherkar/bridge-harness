# fix/project-path-and-new-project-choice — Test Contract

## Functional Behavior

- Reading the repository path for an existing folderless workspace returns a clear domain error that tells the user to connect a folder; it never leaks a SQLite `Invalid column type Null` error.
- Reading the repository path for a connected workspace still returns the stored path.
- Clicking `New project` on the Projects screen opens a choice dialog instead of immediately opening the system folder picker.
- Choosing `Start a chat` closes the dialog and opens a new direct-chat draft.
- Choosing `Choose a folder` closes the dialog and opens the standard directory picker flow.
- Cancelling the dialog leaves the Projects screen and existing state unchanged.

## Unit Tests

- `workspace_path_distinguishes_folderless_and_connected_workspaces` covers both nullable and connected workspace paths in `bridge-core`.
- The App workspace-folder test covers both choices and verifies that the folder picker path is not run before the user chooses it.

## Integration / Functional Tests

- The real `App` composes `ProjectsScreen`, the choice dialog, direct-chat draft navigation, and the existing folder connection API.
- `bun run test` passes across Vitest, Rust, and the Claude sidecar.

## Smoke Tests

- `bun run build` succeeds.
- `bun run check` succeeds.
- Existing project cards, chats, and folder-first composer behavior remain available.

## E2E Tests

N/A — the App-level jsdom tests cover this local desktop interaction without an external service.

## Manual / cURL Tests

- Open Projects, click `New project`, choose `Start a chat`, and verify an empty direct-chat composer opens.
- Open Projects, click `New project`, choose `Choose a folder`, and verify the native directory picker opens.
- Open a folderless workspace and trigger a repository-only action; verify Bridge asks for a folder instead of showing a database error.
