# perf/fast-chat-open-worker — Test Contract

## Functional Behavior

- Conversation and usage loading states use concise reader-facing language without paths or scan diagnostics.
- Usage source paths and diagnostic details stay hidden until the user opens an explicit disclosure.
- Clicking a meter usage bar expands its accessible window detail; clicking it again collapses it.
- Tray and in-app meter entry points open the same meter and trigger the shared refresh behavior.

## Unit Tests

- `AgentConversation.test.tsx` covers the quiet conversation loading state.
- `UsageScreen.test.tsx` covers hidden technical history details and partial-total presentation.
- `MeterPopover.test.tsx` covers meter loading, registered providers, and meter-bar expansion.
- `App.test.tsx` covers tray open, refresh, and window reveal behavior.

## Integration / Functional Tests

- `bun run build` passes.
- `bun run test` passes.

## Smoke Tests

- N/A - the Tauri-specific tray behavior is covered by component integration tests.

## E2E Tests

- N/A - no browser automation surface for the native tray.

## Manual Tests

- Open Usage and select the meter button.
- Click a quota meter bar and verify its detail expands and collapses.
