# Work integrations, past 24 hours — Test Contract

## Functional Behavior
- Work shows a briefing of activity from connected Slack, GitHub, Gmail, Linear, and Notion sources within the rolling past 24 hours.
- Local Bridge checks, approvals, worker queues, and workspace drift never appear, including cached legacy board data and browser preview fixtures.
- Recency uses a timestamp from the cited source item, never the time Bridge read or modified its cache. Unknown, invalid, future, and expired timestamps are excluded, including pinned/hidden legacy tasks.
- Briefing requests contain explicit UTC bounds and ask for recent integration activity only. Unsupported/undated results cannot establish recency; a multi-item response cannot lend one item's date to another.
- Work remains reachable, with an integration-focused heading, source links, honest empty/setup/loading/error states, and refresh that reads integrations when configured.

## Unit Tests
- Source date extraction covers supported provider timestamp formats, unknown/invalid dates, and single-item wrappers without borrowing dates across a collection.
- Board projection excludes internal facts and filters dates before limiting results; source dates survive reconciliation.
- UI ignores legacy facts and stale records, displays dated integration summaries and source links, and expires items while open.

## Integration / Functional Tests
- Source result -> ledger -> committed task -> board preserves activity date separately from observed date.
- Schema migration preserves old tasks but leaves their unknown source dates unset, so they cannot masquerade as recent activity.
- Run `bun run build` and `bun run test`; both must pass before PR creation.

## Smoke Tests
- Browser preview shows an honest empty board, not invented work.

## E2E Tests
- Authenticated live connector runs require the user's signed-in desktop providers; document manual verification if unavailable in this environment.

## Manual Tests
- Open Work, configure a briefing model and connected tools, and refresh. Confirm only activity within the past 24 hours appears with its source link.
- Check a recent Slack message and GitHub update; verify old items do not reappear after refresh or pinning.
- Disconnect a source and confirm errors do not fabricate activity or imply a complete successful read.

## Codex review regressions
- After the asynchronous briefing receipt, keep reading the board while its stored state is `running`; publish terminal results and stop polling without reopening Work. Recover from transient read failures, and do not read an unopened idle board.
- Duplicate reads for the same resource preserve already-earned source dates and links when later results omit them. Later supplied dates/links still update, and both call references continue to resolve to the shared evidence.
