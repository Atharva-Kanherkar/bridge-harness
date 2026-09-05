# Named tool activity — Test Contract

## Functional Behavior
- ACP `tool_call` becomes `tool.started` in `acp_events::tool_call_started`, preserving the provider title and category. Categories outside the recognized read/edit/search/execute/fetch set currently reach the anonymous fallback even with a title.
- A fallback activity with an explicit action title uses that title as its label, during execution and after completion. It must not duplicate the title as a target.
- Known tool names/categories retain their existing labels. Truly unnamed calls retain an honest generic fallback.
- Live and durable events produce the same label for an explicitly titled action.
- Session/turn lifecycle events do not create tool activity. No setup intent is inferred from timing or harness identity; actual startup narration remains separate.

## Unit Tests
- Cover titled ACP `other` calls, whitespace-only titles, named unknown tools, and recognized ACP categories.

## Integration / Functional Tests
- Normalize a first ACP activity and its durable representation and verify the action label survives both paths.
- Verify startup lifecycle events remain non-tool events.

## Smoke Tests
- `bun run build` and `bun run test` pass.

## E2E Tests
N/A — provider sessions are not launched for this code-based investigation; no claim is made that a particular recorded user's first event was reproduced across live providers.

## Manual / cURL Tests
- Review the ACP normalizer and transcript ingestion path against the event fixture used by the regression test.
- In a fresh ACP chat, an `other` tool titled `Resolve project context` should show that action instead of `Using a tool`; after reload the label should be unchanged.
