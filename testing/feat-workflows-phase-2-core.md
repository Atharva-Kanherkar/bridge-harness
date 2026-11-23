# feat/workflows-phase-2-core: Test Contract

Locked before implementation. Phase 2a of Workflows (#845): what makes the
existing feature dependable, event triggers, and templates, in one PR. It
replaces the Phase 2 contract planned in #963. Multi-step workflows (#965),
the repo file (#966) and scheduler hand-off (#968) are out of scope.

## Found while planning (live, before any code)

- **The Slack Inbox has never reached Slack.** Its ingress and render runs
  hand Claude Code an explicit copy of `claude.ai Slack` under
  `strictMcpConfig`, which has no sign-in (the defect #1128 fixed for
  workflow runs). Every 30 s a hidden Sonnet turn reports "no Slack tools
  are available", returns an empty item list, and the poll is recorded as a
  success: the pane says "all caught up". The live store holds 45,153 such
  sessions, zero inbox items, and 26,000+ Claude Code transcript folders.
- Slack event triggers read that ledger, so the ingress fix is part of this PR.

## Decisions

| # | Decision | Why |
| --- | --- | --- |
| P1 | Connector state comes from a stale-while-revalidate cache of Claude's configuration: callers get the last answer at once, and an answer older than 60 s refreshes in the background. Only a cold cache probes inline. Boot warms it. `workflows/refresh_workflow_connectors` probes now. A probe that failed or timed out never replaces a usable answer. Validation, options and per-workflow state read every configured connector (signed in or not), not only the connected ones. | Every save ran `claude mcp list` synchronously (13 to 16 s here, against a 20 s timeout). |
| P2 | Each workflow summary carries the state of each granted connector: `connected`, `needs_sign_in`, `missing`, or `unknown`. A run whose granted account connector is known to need sign-in or to be missing fails before any session starts, with `failure_code = connector_unavailable` and a sentence naming the connector. Unknown proceeds. | Fail fast with a reason instead of a paid run that ends `not_done`. |
| P3 | `WorkflowSettings.maxConcurrentRuns`, default 2, range 1 to 5. A run starts its session only while fewer than that many runs are `running` and no older run waits in line; otherwise it waits as `queued` holding no lease. When a run takes, leaves or gives up a slot, the oldest waiting run starts at once; a run on its way to a slot counts against the free slots. The check and the move to `running` share one store lock, so the cap is exact. `waiting_approval` runs do not hold a slot. Changing to an event trigger, or to another event, repository, channel or sender, needs approval again. | A burst of events or schedules must not start that many Claude sessions on one account. |
| P4 | Each summary carries `averageRunCostMicrousd` (the mean of the last 10 finished, non-dry runs with a known cost) and `monthlyCostEstimateMicrousd` (that mean times the fires of the next 30 days for a schedule, or the runs of the last 30 days otherwise). Both are absent without data. | The person sees what a workflow costs before it surprises them. |
| P5 | `WorkflowTrigger::Event { event, repo?, channel?, from?, batch? }`. Events: `slack_mention`, `slack_direct_message`, `slack_thread_reply` (from the Inbox ledger, no extra model turn); `github_pull_request_opened`, `github_issue_opened` (need `repo`), `github_review_requested` (optional `repo`), read with `gh`, no model turn. `channel` and `from` filter Slack events, case-insensitively, as a substring of the label or author. | The events most people need, from sources Bridge already reads. Gmail, Linear and Notion need their own ingress first. |
| P6 | An event is recorded once per workflow (`UNIQUE(workflow_id, event_key)`). Events that occurred before the trigger's anchor (approval, last change, unpause) never fire. A GitHub source's first read after its anchor is a baseline: what it already shows is recorded as seen and does not fire. | At most one run per event, under replay and across restarts, and turning a trigger on never replays history. |
| P7 | Per-event (default): one run per event, oldest first, one at a time per workflow; events that arrive while a run is active wait rather than being skipped. Batch (`batch: {cron, timezone}`): at each slot, one run with every waiting event (at most 50, the rest wait for the next slot). A slot with nothing waiting starts nothing. | Most people want "collect and run at 9". |
| P8 | An event run's first turn carries its event(s) in a fenced `workflow-event` JSON block, introduced as data that is never an instruction. Just before a run starts, each GitHub event is read in full with one `gh … view` (at most 10 per run): description (at most 4,000 characters), labels, and for a pull request its branches, size and changed files (at most 60). A read that fails leaves the event as it was, with the reason. | Event text is written by other people. A poll reads only what identifies an event, so it stays cheap; live, a PR run that saw only the title could not say what the PR changed. |
| P9 | An event run waits 2 hours for a write approval; every other run keeps 24 hours. | A reply to yesterday's message is stale. |
| P10 | `workflows/preview_workflow_trigger { trigger }` returns `{ supported, reason?, windowDays: 7, matches, samples (at most 5: key, title, url?, occurredAt, payload), note? }`. A dry run may carry one sample (`RunWorkflowParams.event`); a real run may not. | "This would have run 4 times last week", and a dry run on a real past event. |
| P11 | `workflows/list_workflow_templates` returns curated templates, each with the connector families it needs, the families still missing, and a ready definition built with the person's own server ids, default model and timezone. A template is a draft: it is saved unapproved and grants nothing on its own. The Templates tab offers "Try it (dry run)" first and "Customize" second. | The quickest way from install to a working workflow. |
| P12 | Slack Inbox runs load `claude.ai` connectors natively and wait for them. A briefing run whose account connector is not connected ends with a `connector_unavailable` error frame before any turn, and the poll records that as a failure rather than "caught up". Briefing runs get `tools: []`. Connector ingress and render runs do not persist Claude Code transcripts and run on the fast model. Every briefing's turn and cost ceilings reach the SDK (ingress: 6 turns, $0.02). Ingress runs only for a reader: every 2 minutes while the Inbox was read in the last 30 minutes, every 5 while a workflow listens for that family's events, and not at all otherwise; the Slack prompt says to search rather than read channel by channel. (Live: one uncapped read took 27 tool calls and $0.043; at a poll every 2 minutes that is about $30 a day for every Slack user.) A connector probe that failed or timed out is never fresh and never refuses a connector as unknown: validation asks to try again, and options say why the list is empty. | The ingress fix that Slack triggers need, without the cost or the escape tools. |

## Wire contract

- `WorkflowTrigger` gains `Event`. `WorkflowTriggerKind` gains `event`.
- `WorkflowSettings` gains `maxConcurrentRuns` (`#[serde(default = 2)]`).
- `WorkflowSummary` gains `connectors: [{ server, state }]`, `averageRunCostMicrousd?`, `monthlyCostEstimateMicrousd?`.
- `WorkflowRun` gains `event?: WorkflowEventSample` (the first event) and `eventCount`. A run waiting in line is `queued` with no lease; its place comes from the order of `queued` runs.
- `RunWorkflowParams` gains `event?` (dry runs only).
- New methods: `workflows/preview_workflow_trigger`, `workflows/list_workflow_templates`, `workflows/refresh_workflow_connectors` (named in full so each stays unique across domains).

## Storage

Migration 92: `workflow_runs` is rebuilt with `trigger_kind` allowing `event`
and a `trigger_event TEXT` column (JSON array), keeping every row, index and
constraint. New table `workflow_events(id, workflow_id → workflows ON DELETE
CASCADE, event_key, kind, title, url, payload, occurred_at, observed_at,
state ∈ {pending, claimed, baseline}, run_id, UNIQUE(workflow_id, event_key))`.
New table `workflow_event_sources(workflow_id, source, baseline_at)` for the
GitHub baseline.

## Functional Behavior

- F1 Save validates connectors against the cache and returns without a `claude mcp list` when the cache is warm.
- F2 A workflow whose granted connector is cached as needing sign-in shows it on its row, and Run now fails at once with `connector_unavailable`.
- F3 With `maxConcurrentRuns = 1`, two runs claimed together: one runs, one waits `queued` with no lease; when the first finishes, the second starts. No interleaving makes a third run, and the cap holds under parallel dispatch.
- F4 Average and monthly cost estimates follow P4, and are absent without data.
- F5 A Slack mention recorded by ingress after a workflow's anchor creates exactly one event and one run for each matching workflow. A replayed ingress cycle creates nothing. Channel and author filters hold.
- F6 A GitHub PR opened after the anchor fires once. PRs present at the baseline never fire. A restart repeats nothing.
- F7 Batch mode starts one run per slot carrying the waiting events, and none when none wait.
- F8 Events that arrive during an active run wait, and run next.
- F9 The preview counts matches in the last 7 days and returns samples. A dry run with a sample carries it into the run's first turn, with a GitHub event's description and changed files.
- F10 Templates list only connectors the person has; a template with a missing family says which. Try it saves an unapproved workflow and starts a dry run.
- F11 The Inbox ingress reaches Slack (live), reports a disconnected Slack as a failure, never sees a built-in tool, and writes no transcript.
- F12 Validation refuses a malformed event trigger, each case with its reason: unknown event, GitHub repo missing or malformed, batch cron under 15 minutes, filters over 200 characters, and an event trigger on a harness that cannot hold connectors.

## Unit Tests

- Rust `workflows`, `workflow_trigger`, `workflow_events`, `workflow_templates`, `workflow_run`, `marketplace` (cache), `connector_runs_live` (native ingress policy), the migration (rows survive; the CHECK now admits `event`), and protocol round-trips with strict params.
- Property tests (proptest): event filter matching never panics and is case-insensitive; GitHub list parsing never panics on arbitrary JSON; the event ledger never yields two runs for one key, whatever the interleaving of observe, claim and replay; the queue never exceeds its cap.
- Sidecar: briefing options get `tools: []` and native account connectors; the connector wait gates briefing runs; an unavailable connector produces the error frame and no turn; ingress and render runs set `persistSession: false`.
- Frontend: trigger picker, preview, queue position, cost line, connector chips, Templates tab, Try it.

## Mutation testing

`cargo mutants` over the new and changed workflow modules (`workflow_events`, `workflow_templates`, queue and cost code in `workflow_trigger` and `workflows`). Every surviving mutant is either killed by a new test or listed in the PR with why it is equivalent.

## Integration / E2E

- `bridged/tests/workflows.rs`: E5 Slack event per-event and batch (fake SDK, seeded inbox items); E6 GitHub event with a fake `gh` on PATH; E7 queue cap under a burst.
- Live, scratch `bridged`, real Claude: Slack ingress reaches Slack and fills the ledger; a Slack-triggered workflow runs on a real arrival (or a ledger insert when none arrives); a GitHub trigger against a real repo, with preview counts; templates Try it; the queue under a burst of 5 runs; Codex and OpenCode refused with reasons. The desktop UI driven through a browser on the scratch daemon for the editor, preview, Templates and queue.

## Manual / cURL

N/A: no HTTP endpoint changes.
