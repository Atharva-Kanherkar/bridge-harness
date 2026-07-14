# codex/issue-58-marketplace — Test Contract

## Functional Behavior

- Bridge lists available and installed Codex and Claude plugin variants through provider CLI adapters without reading provider credential stores.
- Provider variants group into one service only through explicit aliases or matching repository/publisher, remote MCP endpoint, or verified package metadata; display-name equality alone never merges variants.
- Users can install for Codex, Claude, or both and can enable, disable, update, uninstall, retry, or start provider-native authentication independently per variant.
- A dual-provider install reports each provider result, preserves partial success, and allows retrying only the failed provider.
- Installation, enablement, and authentication remain distinct states. Remote OAuth defaults to separate provider logins; shared authentication is shown only when provider metadata explicitly declares a supported external credential mechanism.
- Native variants are preferred over convertible MCP variants. Provider-specific connectors, hooks, skills, agents, or apps are never marked portable without an explicit mapping.
- Command failures are actionable but redact likely secrets from all surfaced output. Marketplace sources remain visible before installation.
- Existing supervised Codex and Claude sessions continue to use their existing provider configuration unchanged.

## Unit Tests

- Rust catalog parsing accepts common JSON envelopes and retains provider metadata.
- Rust command construction uses provider-native CLI operations and never supplies credentials in arguments.
- Rust error sanitization redacts token-, secret-, authorization-, and key-shaped values.
- TypeScript catalog grouping follows the confidence order and refuses name-only matches.
- TypeScript compatibility classification defaults remote OAuth to separate login and recognizes only explicit shared mechanisms.
- TypeScript dual-install orchestration preserves per-provider success/failure and retries only failures.

## Integration / Functional Tests

- The Tauri marketplace commands expose catalog refresh and per-provider lifecycle actions with structured results.
- The React marketplace loads catalog data, filters/searches grouped services, and invokes provider actions through the API boundary.
- Existing API, conversation, observability, usage, sidecar, and Rust tests remain green.

## Smoke Tests

- `bun run build` completes successfully.
- `bun run test` completes successfully.
- `bun run check` completes successfully.
- With provider CLIs unavailable or returning an error, the marketplace remains usable and shows independent provider errors.

## E2E Tests

N/A — provider marketplace and native authentication flows require locally installed, authenticated interactive CLIs. The UI/API boundary and orchestration are covered by automated functional tests; native prompt completion is verified manually.

## Manual / cURL Tests

- Run `bun run tauri dev`, open Marketplace, and verify Codex and Claude availability/errors render independently.
- Search and provider filters update the service cards without losing provider state.
- Choose each install target and confirm the UI displays distinct install, enablement, and authentication results.
- Trigger **Connect Codex** and **Connect Claude Code** and confirm Bridge starts only the selected provider's native flow and never displays credential material.
- Force one side of a dual install to fail and confirm the successful side remains installed while the failed side alone offers retry.
