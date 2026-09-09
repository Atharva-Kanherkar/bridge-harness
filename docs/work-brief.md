# Work: past 24 hours

Work is a briefing of recent activity from the user's connected integrations. It is
not a task tracker or a dashboard of Bridge's local state. Local completion checks,
approvals, worker queues, and workspace divergence never appear on this screen.

## Opening and configuring Work

Open **Settings → Work briefing → Open Work**. The sidebar remains unchanged.
Settings owns the briefing model, connector selection, optional cadence, and refresh
on focus. The board's **Set up integrations** button returns to that settings page.
Only providers that pass the existing briefing conformance gate can run a briefing.

**Refresh** on a configured board starts a connector briefing through the existing
`work/run_briefing` path. Opening the screen only reads the stored board. While a
run is active, Refresh is disabled. Without a configured model, the screen explains
setup and shows no invented work. The browser preview has no live integrations:
it returns an empty board and refuses a briefing with `desktop_required`.

## What appears

The screen has one list, **Integration activity**, with the newest source activity
first. Each row contains a source/account label, source timestamp, model-written
title and summary, and a source link when a safe permalink was returned. It has no
local task-start, pin, snooze, completion, confidence, or workspace-routing controls.

Supported source families are Slack, GitHub, Gmail, Linear, and Notion. The existing
briefing parser accepts at most 12 summaries per run; the board returns at most the
100 newest eligible saved items. This is a summary of sources actually read, not
an exhaustive event export. Coverage and failure status remain visible.

## The rolling window

A briefing request includes explicit UTC start and end bounds, exactly 24 hours
apart. It asks the model to search within those bounds and fetch each cited item
individually. Collection results are discovery context, not dated item evidence.

The backend derives `sourceActivityAt` from the connector response, independently
of model output:

| Source | Activity timestamp |
| --- | --- |
| Slack | Message `ts`, or its own `edited.ts` |
| Gmail | Message `internalDate` in milliseconds |
| GitHub | `updated_at` / `updatedAt`, or creation time when update time is absent |
| Linear | `updatedAt`, or `createdAt` when absent |
| Notion | `last_edited_time`, or `created_time` when absent |

Single-item JSON/MCP wrappers are unwrapped with a bounded reader. Multiple-item
collections and unparseable or missing timestamps cannot establish recency. A date
on an author, another item, a search result as a whole, or the local cache is not a
substitute for the cited item's date.

`sourceActivityAt` is stored separately from `evidenceObservedAt`, which records
when Bridge read the result. The source date survives the evidence ledger and task
reconciliation. Reading, pinning, or otherwise updating a local record does not
renew its window. Database migration 57 adds nullable source-date columns without
backfilling old rows from observation dates; legacy undated rows remain stored but
are not displayed until a dated source read replaces them.

Both the database projection and the UI exclude future, expired, undated, unknown
source, and non-active records. The database filters and sorts before its row limit.
The open screen advances its clock every 30 seconds so expired items leave the
view without a connector refresh. Failed refreshes retain only saved items that
still qualify for the window.

## Evidence and permissions

The existing read-only briefing authority and strict parser remain in place. A
summary cites successful tool calls from its own run; the model cannot supply its
own source identity, timestamp, or target. The briefing has no shell, filesystem,
or connector-write authority. It never sends messages, changes issues, or starts
work from a summary.

GitHub native `node_id` and `html_url` are recognized alongside the existing field
spellings. Slack workspace permalinks are accepted only under valid, bounded
`<workspace>.slack.com` hostnames. Source links are checked again by Rust when opened.

## Compatibility and verification

The `work/*` protocol, task store, and legacy local action/projection helpers remain
available for compatibility. `WorkBoard.facts` is always empty. The visible board
uses only eligible integration activity, even if a client cache still contains
legacy facts. Persisted local task states are retained, without exposing their old
controls on this surface.

The regression contract is in
[`testing/codex-work-integrations-24h.md`](../testing/codex-work-integrations-24h.md).
Tests cover source formats, safe links, migration, reconciliation, window limits,
legacy cache data, live UI expiry, refresh, setup, and browser preview. Live
connector verification requires signed-in desktop providers.
