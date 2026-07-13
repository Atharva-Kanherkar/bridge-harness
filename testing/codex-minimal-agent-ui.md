# codex/minimal-agent-ui — Test Contract

## Functional Behavior
- A project whose repository path no longer exists is marked unavailable in state instead of allowing operations to fail with a raw `os error 2`.
- Creating a workspace for an unavailable project returns a specific recovery message asking the user to re-add the repository.
- The primary screen is a calm agent conversation with a translucent sidebar, compact workspace header, and secondary controls hidden until needed.
- Live reasoning uses a restrained pulsing indicator; tool calls render as compact activity rows with expandable output.
- Error copy explains what failed and the next recovery action.

## Unit Tests
- Rust stale-project validation test: a deleted repository produces a typed, actionable invalid-project error.
- Existing conversation projection and component tests continue to pass.

## Integration / Functional Tests
- `bun run test` passes.
- `bun run build` passes.
- `cargo check --manifest-path src-tauri/Cargo.toml` passes.

## Smoke Tests
- `http://127.0.0.1:4317/health` returns HTTP 200 with `ok: true`.
- The local app renders with no browser console errors.

## E2E Tests
- Select a surviving workspace belonging to a deleted project root: conversation remains usable and no raw OS error is shown.
- Open the new-workspace flow for a stale project: UI shows a recovery instruction rather than `No such file or directory`.

## Manual / cURL Tests
- Inspect the rendered desktop app at the Vite/Tauri local URL at 1440×900 and 1024×768.
- Verify the sidebar backdrop is visibly translucent and live thinking/tool activity remains readable.
- Run `curl -fsS http://127.0.0.1:4317/health`.
