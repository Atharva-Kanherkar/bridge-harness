# feat/outgoing-sends — Test Contract

## Functional Behavior
- Slack channel, thread and DM drafts and GitHub issue/PR comments and review replies can be sent from their card through the existing connector and GitHub action layers.
- First send call resolves the destination read-only and returns exact text, destination, attachments and a one-use approval token bound to id, revision, resolved destination and SHA-256 of the payload. No write happens. Full access cannot skip approval.
- Approved calls require the issued token and unchanged revision/payload. Editing, fixing, attachment removal, discard or a changed resolution invalidates approval. Denial never writes.
- A durable atomic claim plus the connector in-flight guard prevents concurrent clicks from posting twice. Sent and sending items cannot be sent or edited again. Failed sends show the error and require a fresh approval to retry.
- A remote receipt with id and valid permalink is required before marking sent; turn completion or unverified prose is insufficient. Store receipt and sent_at, publish OutgoingChanged, and record outgoing.sent lifecycle data.
- Slack takes standard Markdown, at most 5,000 characters; thread and DM resolution must be unambiguous. Unsupported uploads are explained on the card with Copy image preserved; images must never disappear silently. Validate pinned image bytes before any upload; use official upload/finalize flow when supported.
- Clipboard destinations keep Copy and have no Send. Existing draft/edit/fix/copy/discard and fork behavior remains valid. Sent copies in forks cannot silently resend.

## Unit Tests
- Outgoing store: schema migration preserves existing drafts; all new states/receipt fields round-trip; approval token binding, missing/forged/replayed/stale tokens, edits/discard invalidation, claims, failure/retry and immutable sent state.
- Connector Post: effect names resolved destination and exact text; prompt treats payload as data; scoped ActionIntent::Post permits only required tools; strict receipt validation rejects missing/wrong ids and URLs.
- GitHub writes: policy approval still required; explicit repository targeting; created id/URL required; mocked CLI verifies comment/reply requests and exact body.
- Protocol: unknown send params refused; registry/params naming, mirror, dispatch, Tauri signature and checked-in artifact gates pass.

## Integration / Functional Tests
- Mocked send executor tests first-call no effect, approved exact payload, concurrent duplicate, failed/denied sends, revision races and event/receipt persistence without real network messages.
- Frontend card tests approval/cancel/send, stale dialog revision, double click, sending/sent/permalink, failed/retry, unsupported images and read-only/archive behavior.
- Frontend API mock exercises approval and send lifecycle.

## Smoke Tests
- `bun run build` completes.
- `bun run test` completes; report any pre-existing or environment failures with evidence rather than claiming success.

## E2E Tests
- Fresh browser profile against local dev server: open mock conversation, show Send approval, cancel without state change, approve to sent receipt, inspect rendered status and permalink; verify light and dark chrome.
- Live Slack/GitHub messages are not sent as tests without an explicit real destination from the person. Use mocked provider integration coverage instead.

## Manual / cURL Tests
- In a connected desktop chat, draft to a real Slack thread, edit, press Send, verify exact effect and attachment listing, cancel once, then approve; inspect receipt. Repeat for GitHub comment. Requires a user-chosen real destination and remains a documented manual check.
- Unconnected Slack or unavailable upload tools: show a clear reason and preserve Copy/Copy image.
