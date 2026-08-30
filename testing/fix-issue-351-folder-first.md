# fix/issue-351-folder-first — Test Contract

## Functional Behavior

- Selecting **New project** opens the native directory picker immediately; no workspace-name dialog is shown.
- Cancelling the picker does not call `createWorkspace` and leaves the state unchanged.
- Choosing a folder creates one workspace titled with that folder's leaf name, then connects that folder to it.
- Choosing a folder already recorded on a workspace does not create another workspace; the existing workspace becomes the selected project.
- A non-Git folder is passed through the existing `connectWorkspaceFolder` path, which continues to support it.
- The existing pathless-workspace UI remains available for already persisted rows.

## Unit Tests

- `newWorkspaceFromFolder` behavior is covered through the App handler's API calls: it creates a leaf-named workspace only after a string folder selection, then connects that new workspace.
- Existing project lookup recognizes a matching connected folder and selects it without calling `createWorkspace`.

## Integration / Functional Tests

- `ProjectsScreen` still dispatches the New project action when its button is pressed.
- Existing Rust workspace tests continue proving `connect_workspace_folder` accepts plain folders and resolves Git repositories.

## Smoke Tests

- `bun run build` completes successfully.
- `bun run test` completes successfully.

## E2E Tests

N/A — native directory-picker interaction is not available in the jsdom test environment.

## Manual Tests

1. Open Projects and press **New project**; confirm the directory picker opens directly.
2. Cancel it; confirm no project appears.
3. Select a plain folder; confirm one connected, leaf-named project appears.
4. Press **New project** again and select that same folder; confirm it selects the existing project without adding a duplicate.
