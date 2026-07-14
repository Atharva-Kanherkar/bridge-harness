# feat/claude-agent-sdk-sidecar — Test Contract

## Functional Behavior

- A fresh Claude sidecar query must use the provider session UUID allocated by the Rust adapter, so SDK output and the persisted `provider_session_id` agree.
- A resumed query must pass the persisted UUID through the SDK `resume` option and must not also request a new session ID.
- Claude availability must depend on the runtime actually used by chat (`node` plus the sidecar entrypoint), not on a separately installed global `claude` CLI.
- Existing write-mode permission mappings and multi-turn stdin/stdout framing must remain unchanged.

## Unit Tests

- Sidecar option construction preserves `sessionId` for fresh queries.
- Sidecar option construction uses `resume` without `sessionId` for resumed queries.
- Sidecar option construction preserves isolation, MCP, system-prompt, and permission settings.
- Rust write-mode labels and explicit sidecar-path resolution continue to pass.

## Integration / Functional Tests

- `bun run build` succeeds.
- `bun run test` runs the sidecar tests, frontend tests, and Rust tests successfully.
- `cargo test --lib claude_adapter` succeeds.
- `node --check sidecar/claude-agent/index.mjs` succeeds.

## Smoke Tests

- A one-turn live sidecar query emits the configured session UUID in its `system/init` and `result` frames.
- A second turn can be sent through the same long-lived sidecar process without spawning a replacement process.

## E2E Tests

- Manual in-app Claude chat remains the final E2E check because it requires a local authenticated Claude account and desktop UI.

## Manual / cURL Tests

- N/A — this is a local process integration, not an HTTP endpoint.
