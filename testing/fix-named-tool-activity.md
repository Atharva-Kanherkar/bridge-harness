# Named tool activity — Test Contract

## Functional Behavior
- ACP `tool_call` becomes `tool.started` in `acp_events::tool_call_started`, preserving the provider title and category. Categories outside the recognized read/edit/search/execute/fetch set currently reach the anonymous fallback even with a title.
- A fallback activity with an explicit action title uses that title as its label, during execution and after completion. It must not duplicate the title as a target.
- Known tool names/categories retain their existing labels. Empty unnamed pending/running calls stay in the transcript reduction but wait for an action name or output before appearing as activity. Unnamed terminal results retain an honest generic fallback.
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

## Review revision contract
- The terminal fallback is provider-neutral: trimmed titles render as `Running: <title>` / `Finished: <title>` without a duplicate target.
- ACP `think` and `switch_mode` have explicit tense pairs and meaningful icons.
- A real ACP start followed by title-less raw progress/completion updates retains its title through live reduction and durable replay. Replay must not synthesize a replacement tool title.
- A dedicated golden fixture covers this sequence and render assertions distinguish running from finished text.
- Remove redundant lifecycle assertions and the unrealistic completion codec test.

## Anonymous start follow-up
- Suppress only empty pending/running tool calls with no tool name, recognized action category, or action title. Do not create an empty activity group or count the placeholder as work beside a named call.
- A progress title updates the existing item; its identity and ordering stay intact, and subsequent title-less updates keep that name.
- Output, completion, and failure reveal the retained call even if it is still unnamed. Named tools, reasoning, and unknown-event reduction retain their existing behavior.
- Regression tests cover hidden anonymous starts and visible terminal results through both live and durable projections, plus live late-title adoption and output arrival. Existing named ACP golden/render tests protect stored-title replay parity.
- Evidence boundary: the anonymous ACP event is adapter-shaped test data, not a captured event from the original fresh-chat report. No live paid provider was launched, so this fix does not identify that report's exact startup frame or prove provider-wide reproduction.
- Existing durability limit: a title supplied only on transient `tool.progress` is not persisted by the forest. If neither a start nor a terminal frame includes it, a reload cannot recover that name. This frontend change preserves parity for the data actually stored; persisting progress-only metadata is outside this change.
