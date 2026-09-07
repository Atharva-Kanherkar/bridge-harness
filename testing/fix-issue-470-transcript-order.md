# fix/issue-470-transcript-order — Test Contract

## Functional Behavior

- A transient, coalesced assistant delta retains the causal position of its first arrival.
- If a persisted tool frame arrives between two chunks of that assistant delta, the assistant reply renders before the tool.
- Durable events retain their persisted sequence ordering.

## Unit Tests

- `appendAgentEventBatch` preserves a transient item's first-arrival causal anchor when a later delta is merged.
- `reduceConversation` orders the real buffered straddle scenario causally.

## Integration / Functional Tests

- The reducer test builds its input through `appendAgentEventBatch` and then calls `reduceConversation`.

## Smoke Tests

- `bun run build`
- `bun run test`

## E2E Tests

N/A — this reducer and buffer behavior is covered by unit/integration tests.

## Manual Tests

- In a streaming conversation, verify that text which begins before a tool call stays above that tool while later text chunks arrive.
