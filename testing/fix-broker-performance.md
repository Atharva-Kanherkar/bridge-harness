# Brokered Credential Use and Responsive Streaming — Test Contract

## Functional Behavior

- A detected OpenAI credential is replaced before provider transport, UI projection, or persistence.
- The wrapped agent is explicitly told that an opaque credential reference is safe to use through Bridge and must not be treated as a pasted or exposed key.
- When a user asks to "call", "test", or "verify" an OpenAI reference without naming an operation, the agent is directed to verify it with `GET /v1/models` through the Bridge proxy.
- Non-OpenAI secret families remain redaction-only and are never represented as callable credentials.
- Provider frames that do not mutate global session/workspace state do not trigger a full `get_state` reload.
- Repeated forest snapshots that have not changed do not replace React state or rebuild the conversation.
- Conversation projection/grouping is memoized, and ambient background animation pauses while an agent turn is active.

## Unit Tests

- Credential broker instructions describe references as authorized capabilities, define safe default verification, and prohibit generic secret-exposure warnings.
- Credential interception metadata distinguishes brokered OpenAI references from redaction-only references.
- Backend state-change classification returns true only for events that mutate global state.
- Frontend forest snapshot identity detects unchanged and changed snapshots.

## Integration / Functional Tests

- Preparing and sending an OpenAI credential retains only the sanitized reference while registering a session-bound broker record.
- A representative secret-use prompt produces agent-facing text that identifies the reference as an OpenAI capability without containing the credential.
- Existing conversation and observability reducers continue to pass their test suites.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds, including Rust tests.
- `bun run check` succeeds.
- The Tauri development app launches and its health endpoint remains responsive.

## E2E Tests

- Automated provider-level E2E is not deterministic in CI because it requires signed-in Codex/Claude subscriptions. The regression contract tests the exact agent-facing instruction and proxy boundary instead.

## Manual / cURL Tests

- In a fresh or resumed Codex chat, paste a disposable OpenAI test credential and ask "verify this key". Confirm the visible turn contains only `[secret:sec_…]` and the agent calls Bridge's `/v1/models` proxy route instead of refusing.
- Repeat with Claude.
- Start a streaming turn in a long conversation and confirm typing, scrolling, and window movement remain responsive while background animation is paused.
