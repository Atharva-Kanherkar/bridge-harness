# Eval: connector render runs

The connector surface asks a harness to read a real account and emit a card.
Whether that works is not a question unit tests can answer — it depends on a
live MCP server, a real thread, and a model following a prompt. This is the
procedure for checking it, and for turning what it finds into a regression test
that does run in CI.

## What is being evaluated

One property, in two halves:

1. **Followability** — a render run pointed at a real message emits exactly one
   `bridge-connector-card` fence containing a card that passes
   `ConnectorCard::validate`.
2. **Degradation** — when it does not, the item still gets a card and the user
   still gets a notification. A render failure may cost polish. It may never
   cost the notification.

The second half is the one that matters more, which is why the recorded
fixtures are mostly failures.

## Preconditions

```bash
claude mcp list | grep -i slack
#   expect: claude.ai Slack: https://mcp.slack.com/mcp - ✔ Connected
```

`! Needs authentication` means the connector is signed out. Sign in from the
harness — Bridge holds no Slack credential and cannot fix this for you.

## Running it live

1. Launch Bridge with the harness whose MCP configuration holds the connector.
2. Open the **Inbox** dock pane. The ingress timer runs every 30s; the refresh
   control in the pane header runs one cycle immediately.
3. Have someone send you a DM, or @-mention you in a channel you are in.
4. Watch for, in order:
   - a toast within one poll cycle, in Bridge's own wording (shimmering);
   - the same toast's headline sharpening once the render run lands;
   - the item in the pane with a card and, usually, suggested replies.

### What each outcome means

| What you see | What it means |
| --- | --- |
| Toast, then a sharpened headline | Both halves pass. |
| Toast that never sharpens, card marked "Bridge wrote this card itself" | Followability failed, degradation held. **Record the run as a fixture.** |
| No toast at all | Ingress failed. Check the pane's degraded badge — it carries the reason. |
| "You're all caught up" while a message is genuinely unread | The ingress prompt is not finding that message class. A contract bug, not a rendering one. |
| Toast for a message you already answered | Dedup broke. This is the most serious failure on this list. |

## Recording a fixture

A failed render is only worth finding once. To make it permanent:

1. The run's session is hidden but real — `kind = 'connector'` in `sessions`.
   Read its assistant text out of the session forest.
2. Save the **whole assistant message**, not just the card, to
   `testing/fixtures/connector-render/<shape>.txt`. Half of what the replay
   tests is the extraction: prose around the fence, two fences, a truncated one.
3. Replace any real names, channels, or message bodies with synthetic ones of
   the same shape and length. These fixtures are committed; a stranger's message
   is not ours to commit.
4. Name it in `connector_eval::tests::every_recorded_fixture_is_exercised_by_this_module`
   and add an assertion for what it should do. That test fails on an unnamed
   fixture on purpose — a fixture nobody replays silently stops meaning anything.

```bash
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core --lib connector_eval
```

## Cost

Every card is a model turn, so this surface has a per-run cost ceiling
(`connector_runs::run_limits`) rather than a best-effort budget. A run that
would exceed one is stopped and the item falls back to the card Bridge writes
for nothing. When evaluating, watch that the ceilings are not being hit
routinely — that is the signal that the render prompt is doing too much, not
that the ceiling is too low.
