# Cursor dashboard history in Usage

The main Usage screen and native Menu Bar use Bridge's shared Cursor collector.
The History sources section identifies Cursor as **Dashboard usage** and shows
cached event counts and the original update time. Scan history refreshes the
enabled Cursor provider through the same bounded collector and refresh gate as
the menu. Listing sources and switching report metrics never fetch credentials
or contact the dashboard.

Cursor's own local chat and tracking databases do not supply token counts. The
local importer still reports that limitation accurately. The desktop API
substitutes the authenticated dashboard source for that unsupported local row;
it does not manufacture transcript observations from account totals.

During the existing dashboard request, Bridge retains exact daily/model
aggregates alongside the menu snapshot: uncached input, cache read, cache write,
output, event count, known cost and unpriced event count. Fractional cents are
summed before rounding to micro-USD. Missing cost remains unpriced, and session
counts are null because dashboard events do not identify sessions. Cache savings
in the main report are explicitly local estimates.

The main report selects Cursor dashboard history as its exclusive Cursor source
when imported history is enabled and the request is for daily, unscoped totals in
the cache's time zone. Device-local Cursor rows are excluded before aggregation,
including their request counts. They cannot be added to an account total without
proof of account ownership. Claude, Codex and OpenCode retain their existing
live-versus-transcript deduplication.

Dashboard data covers the last 30 calendar days through its exact observation
time. Today is an as-of snapshot; activity since that observation appears after
refresh. Longer ranges are marked incomplete. Hourly views, workspace attribution
and a different time zone cannot be reconstructed from daily aggregates; the UI
states this instead of assigning tokens to invented times or workspaces.

The cache survives restarts. Transient history failures can retain it only for an
exact matching verified account scope, with its original timestamp and stale
state. Missing or changed account identity cannot expose another account's
history. An older cache without the exact breakdown remains usable by the menu
and asks for a refresh before participating in the main report. A successful
empty dashboard response is distinct from unavailable history.

Protocol 1.16 adds the local/dashboard source origin and the optional
`includeDashboard` request flag. Only opted-in summaries can contain dashboard
buckets with unavailable session counts. Older clients keep local-only summaries
with integer counts, while the updated client rejects older daemons that cannot
serve the requested dashboard data.

## Claude: which source is better?

| Question | Correct authority | Bridge and CodexBar |
| --- | --- | --- |
| How much of the five-hour, weekly or Fable allowance is used? | Anthropic account usage (OAuth; explicit CLI fallback in Bridge) | Account-wide limits. Transcript tokens cannot reconstruct these percentages. |
| How many tokens did Claude Code use on this Mac, by model and day? | Local transcript usage metadata | Both scan Claude JSONL. These records can span accounts and do not cover other devices. |
| What is the API-equivalent cost? | Reported event cost, otherwise known model prices applied to recorded token buckets | An estimate when computed from rates, never a Claude subscription bill. Unknown prices stay unknown. |

Bridge's existing ledger is the right shared authority for its two surfaces: it
deduplicates live Bridge sessions against complete imported transcripts, keeps
cache token components separate and reprices history consistently. The Menu Bar
adds current account quotas to that same historical data; it does not maintain a
second Claude token scanner.

CodexBar has more extensive scanner caching, cancellation, stale fallback and
account-ownership handling. Those are useful future improvements to Bridge's
scanner, not reasons to replace transcript history with OAuth utilization or
relabel device history as belonging to the currently signed-in account.

Reference study: CodexBar `CursorStatusProbe.fetchCostReport`,
`CursorUsageEventsFetcher`, `CostUsageFetcher`, `CursorLocalCSVReader`,
`CostUsageScanner` and the Claude OAuth credential/usage paths. Its Cursor cost
selection prefers a remote report, with third-party tokscale CSV as a fallback;
the two sources are not added together. Bridge reuses its one persisted provider
collector rather than adding a second dashboard fetch to the History screen.
