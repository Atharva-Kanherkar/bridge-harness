# feat/active-turn-settings Test Contract

## Functional Behavior
- Settings > General offers Steer and Queue for all chats, persisted across restart and applied immediately.
- Steer delivers into a capable provider's live turn; unsupported steering falls back to queue with a setting note and send tooltip. Queue waits for the turn boundary without interrupting.
- Idle messages start a turn in either mode. Legacy clients omitting the new field retain their current interrupt behavior.
- Existing queued messages remain visible when the mode changes. Unsupported queued images fail explicitly.
- Archived chats is reachable from Settings > Data & storage, with Settings navigation visible, and no longer has a sidebar shortcut.

## Unit Tests
- sessionInput: both preferences crossed with present, absent, and undefined steering capabilities.
- Preference storage: missing/invalid values default to steer, queue survives reading again.
- Protocol: optional preference compatibility, enum round trip, rejection of invalid values.

## Integration / Functional Tests
- Settings control updates stored preference and mounted consumers immediately.
- Backend: Queue on a steering-capable active session does not deliver or interrupt; Steer delivers natively; unsupported Steer queues; idle starts normally.
- Archive Settings navigation is discoverable.

## Smoke Tests
- bun run build and bun run test pass.

## E2E Tests
N/A: native provider sessions verified with Rust runtime fixtures and component tests.

## Manual Tests
- In Settings choose Queue, send during an active turn, and observe Queue plus queued count; switch to Steer and check the button updates.
- Restart and confirm the selected mode survives.
- Open Data & storage > Archived chats and check the settings rail and back link remain visible.
