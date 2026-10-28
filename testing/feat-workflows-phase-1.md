# Workflows, Phase 1: Test Contract

Contract for Phase 1 of the Workflows epic (#845), written before any implementation and
reviewed against, not after. Issue #846 owns this file. Slices #847 to #853 implement it.
A requirement that changes during implementation changes here first, in its own commit.

Phase 1 in one sentence: a user, with or without a project, describes a repeatable job,
approves exactly what it may touch, and Bridge runs it on a schedule or on demand through
their own harness, asking before every write, and reports the result in one inbox.

## Decisions closed by this contract

| # | Question | Decision | Why |
| --- | --- | --- | --- |
| C1 | Session kind and visibility | A run is a session with `kind = 'workflow'`. It joins the hidden session kinds (the core predicate and the sidebar's list), so it is not listed in the chat sidebar, chat search, memory extraction or routing evaluation, but it stays in Bridge's state and is openable by id from the Workflows page and the inbox, read-only. | `is_hidden_session_kind` means "never visible", which is wrong for runs a user must inspect. Runs carry untrusted connector text, so they stay out of memory and search, like briefing sessions. |
| C2 | Can a user type into a run? | No. Core refuses a user turn to a `workflow` session. "Continue in chat" forks it into a normal direct chat with `sessions/fork_session`. | A follow-up turn would otherwise run with the workflow's grants and no review. |
| C3 | Schedule format | A 5-field cron expression plus an IANA timezone. The UI offers presets that compile to cron. Minimum gap between two consecutive fire times is 15 minutes. | Claude Code's native tasks use 5-field local-time cron (`automations.rs`), so the Phase 2 hand-off is a copy. 15 minutes is the floor the learning schedule and Work briefings already enforce. |
| C4 | Daylight saving | A local time that does not exist (spring forward) fires once at the next valid instant. A local time that occurs twice (fall back) fires once, at the first occurrence. | One fire per intended occasion, never zero, never two. |
| C5 | Missed slots | On boot or wake, only the latest missed slot runs, once, with `triggerKind = 'catch_up'` and `missedSlots = n`. Earlier missed slots are not run. A slot claimed more than 5 minutes after its time counts as a catch-up. | Matches Claude Desktop's local tasks; a burst after a weekend away would drain the plan. |
| C6 | Which harness may run what | Connector grants, and `files: "none"`, require an adapter that can enforce exact tool identities (`briefing_policy::adapter_may_brief`): Claude today. Codex and OpenCode are offered only for project-bound `files: "worktree"` workflows with no connector grants, and are shown disabled with the gate's own reason otherwise. | The gate already says Codex has "no per-tool authority" and OpenCode's rules "are coarse families". A workflow cannot promise "granted tools only" on an adapter that cannot enforce it. |
| C7 | Built-in tools | `files` is `none` or `worktree`. `none`: built-in file, shell and web tool families are denied, as in a briefing run. `worktree` (project-bound only): the run gets its own worktree, prepared from the project's folder and registered like any chat worktree, under the existing Isolated write mode, and carries no connector grants in Phase 1. A no-project workflow is always `none`; its home folder is its working directory only. | Keeps Phase 1 to two shapes the policy engine already enforces. |
| C8 | Write approvals on Claude | The Claude sidecar gains an ask path. Any call on a server granted `write: "ask"` that is not a read on a read-granted server makes `canUseTool` emit a `control_request` (`can_use_tool`) and wait; Bridge already normalizes that to `permission.requested` and answers with the existing `control_response`. Rust decides first: a duplicate is declined, a dry run is declined with a preview, an `alwaysAllow` identity is accepted; anything else shows the card and the person answers with `approvals/resolve_approval`. Every decision is recorded as an effect. On teardown, a pending ask resolves to decline. | The sidecar has no human-ask path today (chat modes are `acceptEdits` or `bypassPermissions`). Reusing `approval.requested` keeps one approval surface. |
| C9 | Always allow | "Always allow for this workflow" is one core call, `workflows/always_allow_tool`. It writes a new revision with that exact tool identity in `alwaysAllow` and accepts the pending approval, atomically. | Two separate calls could accept without recording, or record without accepting. |
| C10 | Full access | Chat-level Full access never applies to workflow runs. A write grant without always-allow always asks. | Workflow writes are outward effects; outward effects keep asking even under Full access, as browser outward effects do. |
| C11 | Quota reserve | Default 25 percent, range 0 to 90, in Workflows settings. A run does not start while any usage window of its provider is at or above `100 - reserve` percent used; it waits as `waiting_quota` until that window resets. When the provider reports no usage, the run proceeds and is labelled "quota unknown". | Your own chats come first. Unknown usage cannot be allowed to block every workflow forever, so it is shown, not hidden. |
| C12 | Expected result | Instructions end with a request for a fenced `workflow-result` JSON block: `{"status":"done"\|"not_done","summary":string}`. Core parses it. `done` with no failed write is `succeeded`; `not_done` is `failed`; a missing or invalid block is `unverified`. No model judges the outcome. | "Succeeded" must mean the job reports done, not that the session ended. A deterministic parse needs no eval. |
| C13 | Duplicate writes | An executed write is recorded with key `sha256(runId, toolIdentity, canonicalArguments)`. A resumed or retried run that issues the same call is denied as a duplicate. Two identical writes in one run are therefore impossible, by design. | At most one effect per approved write, across crashes. |
| C14 | Draft eval | P1.5 ships 30 recorded sentences with expected trigger and grants. Replay runs in `bun run test`. A live run is required before P1.5 merges, with results in the PR. Bar: zero over-grants (hard gate), at least 27 of 30 triggers exact, at most 3 under-grants. | Drafting is a model decision; a grant it invents is a permission the user did not mean to give. |
| C16 | Grant granularity | Connectors are granted per server: `read` admits that server's read-verb tools (the briefing scoped-read rule), `write: "ask"` lets the run ask before any other tool on that server. Exact identities appear only in `alwaysAllow`. A tool's class comes from the briefing read-verb vocabulary at call time and fails closed to write. | A harness only presents its tool list once a session starts, so exact tools cannot be offered or validated at save time. Server-level grants are what Bridge can know and enforce before a run. |
| C15 | Settings storage | `configuration_entries` under `(kind = 'workflows', id = 'settings')`: `paused`, `quotaReservePercent`. | The same pattern Work settings use. |

## Wire contract

Domain `workflows`. Every method follows the protocol checklist: registry triple in
`methods.rs`, params with `deny_unknown_fields` named after the method suffix, the typed
table in `messages/mod.rs`, a `bridged/src/dispatch.rs` arm, a body in `bridge_core::api`,
a Tauri command in `generate_handler!`, regenerated artifacts.

| Method | Params (camelCase on the wire) | Result | Slice |
| --- | --- | --- | --- |
| `workflows/list_workflows` | none | `{ workflows: WorkflowSummary[] }`; P1.2 adds `nextRunAt` and `lastRun` to each summary | P1.1 |
| `workflows/get_workflow` | `{ workflowId }` | `{ workflow: Workflow }` | P1.1 |
| `workflows/save_workflow` | `{ workflowId?: string, definition: WorkflowDefinition }` | `{ workflow: Workflow, needsApproval: boolean }` | P1.1 |
| `workflows/approve_workflow` | `{ workflowId, revisionId }` | `{ workflow: Workflow }` | P1.1 |
| `workflows/delete_workflow` | `{ workflowId }` | `{}` | P1.1 |
| `workflows/set_workflow_enabled` | `{ workflowId, enabled: boolean }` | `{ workflow: Workflow }` | P1.1 |
| `workflows/workflow_options` | none | `{ harnesses: WorkflowHarnessOption[], servers: WorkflowServerOption[] }` | P1.1 |
| `workflows/read_workflow_settings` | `{}` | `{ settings: WorkflowSettings, configured: boolean }` | P1.2 |
| `workflows/write_workflow_settings` | `{ settings: WorkflowSettings }` | `{ settings: WorkflowSettings }` | P1.2 |
| `workflows/run_workflow` | `{ workflowId, requestId, dryRun: boolean }` | `{ run: WorkflowRun, created: boolean }` | P1.2 |
| `workflows/cancel_workflow_run` | `{ runId }` | `{ run: WorkflowRun }` | P1.2 |
| `workflows/list_workflow_runs` | `{ workflowId?: string, unreadOnly: boolean, limit: number }` | `{ runs: WorkflowRun[] }` | P1.2 |
| `workflows/always_allow_tool` | `{ workflowId, sessionId, eventId, toolIdentity }` | `{ workflow: Workflow, resolution: InteractionResolutionResult }` | P1.3 |
| `workflows/mark_workflow_runs_read` | `{ runIds: string[] }` | `{}` | P1.6 |
| `workflows/draft_workflow_from_text` | `{ text, projectId?: string \| null }` | `{ draft: WorkflowDraft }` | P1.5 |
| `workflows/draft_workflow_from_session` | `{ sessionId }` | `{ draft: WorkflowDraft }` | P1.5 |

Notifications, both `Transient`, named in the newer `domain/event` style:

- `workflows/changed`: `{ workflowId }` after save, approve, enable, disable or delete.
- `workflows/run_changed`: `{ run: WorkflowRun }` on every run status change.

Shapes:

```text
WorkflowDefinition {
  name: string                                  // 1..=80 chars, trimmed
  projectId: string | null                      // null = no project
  steps: [WorkflowStep]                         // exactly 1 in Phase 1
  trigger: { kind: "manual" }
         | { kind: "schedule", cron: string, timezone: string }
  grants: [{ server: string, read: boolean, write: "never" | "ask" }]
  alwaysAllow: [string]                         // exact mcp__<server>__<tool> identities
  files: "none" | "worktree"                    // "worktree" needs a projectId
  budget: WorkBriefLimits                       // { maxWallSeconds, maxTurns, maxToolCalls, maxOutputTokens?, costCeilingMicrousd? }
  expectedResult: string                        // shown to the agent and the user
  notify: { onSuccess: boolean }                // failures and approvals always notify
}
WorkflowStep { harness: string, model: string, effort?: string, instructions: string }
WorkflowRun {
  id, workflowId, revisionId, triggerKind: "manual" | "schedule" | "catch_up",
  missedSlots: number, dryRun: boolean,
  status: "queued" | "waiting_quota" | "running" | "waiting_approval"
        | "succeeded" | "unverified" | "failed" | "cancelled" | "skipped",
  failureCode?: string, summary?: string, sessionId?: string,
  quotaUnknown: boolean, costMicrousd?: number, tokens?: number,
  unread: boolean, scheduledFor?: string, startedAt?: string, finishedAt?: string
}
```

Budget bounds, refused with a reason and never coerced: `maxWallSeconds` 60..=3600,
`maxTurns` 1..=64, `maxToolCalls` 1..=256, optional ceilings strictly positive. The same
bounds Work briefings use.

## Storage

One new migration (the next `LATEST_SCHEMA_VERSION`, 67 at the time of writing).

- `workflows(id PK, name, project_id NULL REFERENCES projects(id), enabled, current_revision_id, disabled_reason NULL, created_at, updated_at)`.
- `workflow_revisions(id PK, workflow_id, revision, schema_version, definition JSON, digest, approved_at NULL, created_by, created_at, UNIQUE(workflow_id, revision))`. `created_by` is one of `user`, `draft_from_text`, `draft_from_session`, `always_allow`. Rows are never updated except `approved_at`, set once.
- `workflow_runs(id PK, workflow_id, revision_id, trigger_kind, occasion, idempotency_key UNIQUE, missed_slots, dry_run, status, failure_code NULL, summary NULL, session_id NULL, lease_owner NULL, lease_expires_at NULL, quota_unknown, cost_microusd NULL, tokens NULL, unread, scheduled_for NULL, started_at NULL, finished_at NULL, created_at)`, with a partial unique index on `workflow_id` where `status IN ('queued','waiting_quota','running','waiting_approval')`. The database itself forbids two active runs of one workflow.
- `workflow_run_effects(id PK, run_id, effect_key UNIQUE, tool_identity, arguments_digest, decision, preview NULL, decided_by NULL, created_at, executed_at NULL)`. `decision` is one of `previewed`, `approved`, `always_allowed`, `declined`, `executed`, `failed`.
- Idempotency key: `schedule:<workflowId>:<slot RFC 3339 UTC>` for schedules and catch-ups, `manual:<workflowId>:<requestId>` for Run now.
- Lease: owner and expiry are both set or both null, as in `work_brief_store::begin_run`.
- No-project home: `<data_dir>/workflows/<workflowId>/`. Never a `projects` row.

## Functional Behavior

### Saving (P1.1)

- F1. A workflow saves with `projectId: null` or an existing project id and reads back byte-identical in its definition.
- F2. Every save creates a new immutable revision; the old revision is unchanged and still readable.
- F3. A revision that widens authority has `approved_at = null`. Widening is any of: a new server, `read` false to true, `write` never to ask, a new `alwaysAllow` identity, `files` none to worktree, a different harness, a different project. A revision that only narrows or rewords keeps approval. `always_allow_tool` revisions are approved by construction (C9).
- F4. Saving a no-project workflow twice leaves one home folder. Deleting the workflow removes it. No `projects` row is created at any point.
- F5. A project-bound workflow whose project row or folder is gone is disabled with `disabled_reason = 'project_removed'` the next time Bridge reads or claims it; it is never orphaned or run somewhere else. (Bridge has no project-removal path today, so the check runs on read and claim.)
- F6. Refused, each with a reason: more than one step; `files: "worktree"` without a project; connector grants on an adapter that fails C6; a cron that does not parse; consecutive fire times under 15 minutes apart; an unknown timezone; a budget outside its bounds; a grant on a server the harness's configuration does not hold; a duplicate server; an `alwaysAllow` identity that is not `mcp__<server>__<tool>` on a server granted `write: "ask"`; connector grants together with `files: "worktree"` (C7).
- F7. `workflow_options` lists every registered adapter with whether it may hold connector grants and whether it may run worktree jobs, each with the gate's reason when refused, and every connector server the Claude configuration holds with its family and connection state. `workflows::tool_class` classifies a tool identity as read or write with the briefing read-verb vocabulary and fails closed to write.

### Triggering (P1.2)

- F8. One entrance, `workflow_trigger::claim`, for manual, schedule and catch-up. Nothing else starts a run.
- F9. Firing the same occasion twice yields one run; the second caller gets the existing run with `created: false`.
- F10. A claim while the workflow has an active run inserts a `skipped` run with `failure_code = 'overlap'` for schedules, and returns the active run for Run now.
- F11. A crash after claim: once the 15-minute lease expires, a CAS reclaim marks the run `failed` with `lease_expired`. A worker whose lease was reclaimed cannot write a result.
- F12. C4 and C5 hold under a fake clock.
- F13. Disabled, unapproved, project-removed and globally paused workflows never claim on a schedule, and Run now on them is refused with the reason. One exception: a manual dry run is allowed on an unapproved workflow, because a person started it and every write becomes a preview.
- F13a. A schedule counts slots from its anchor: the latest of its last scheduled run's slot, the current revision's approval, the workflow's last change, and the moment Workflows were last unpaused. Slots before the anchor are never caught up, so turning a workflow back on does not replay the time it was off.
- F14. The engine reads only the `workflows` tables. Native Claude Code and Codex schedules are never dispatched by it.
- F15. The maintenance loop runs in the daemon host and in the embedded host. Closing the desktop window does not stop schedules under the daemon host.

### Running (P1.3)

- F16. A run creates a `workflow` session. Project-bound with `files: "worktree"`: a new task worktree through the normal workspace path. Otherwise: no workspace, cwd = the workflow home.
- F17. The instructions compile as a purposed prompt, followed by the expected result and the C12 result-block request. Trigger payloads and connector results are framed as data.
- F18. On Claude, the sidecar gate allows read-class tools on servers granted `read`, allows identities in `alwaysAllow`, asks for any other tool on a server granted `write: "ask"`, and denies everything else with a reason naming the tool. Nothing is pre-approved through `allowedTools`. MCP servers are filtered to granted servers with `strictMcpConfig`, and no setting sources or plugins are inherited, as in `briefingOptions`.
- F19. A dry run denies every write and stores a `previewed` effect with the attempted arguments.
- F20. Before dispatch the quota rule (C11) is checked; mid-run, hitting any budget ceiling stops the run as `failed` with `budget_exhausted`.
- F21. A write approved or always-allowed is recorded before it executes; a second identical call in that run is denied as a duplicate (C13).
- F22. Teardown, cancel or lease loss resolves any pending ask to decline and settles the run.
- F23. Outcome follows C12. Usage lands in `usage_ledger` tagged with the run id and is summed onto the run.
- F24. Core refuses a user turn to a `workflow` session (C2).

### Workflows page (P1.4)

- F25. The sidebar shows "Workflows" where "Scheduled tasks" was; the `automations` view and its title are gone.
- F26. Native Claude Code and Codex schedules are listed in a "Native schedules" section of the page with every action the old panel had.
- F27. A no-project workflow can be created, approved, run and opened without choosing a folder.
- F28. Harnesses that fail C6 for the current grants are shown disabled with the reason.
- F29. A run opens read-only with its tool calls, effects and cost; "Continue in chat" forks it.

### Drafting (P1.5)

- F30. A draft is never persisted until the user saves the prefilled form.
- F31. A sentence naming an app with no connected server yields a "connect X first" note, not a grant.
- F32. "Save as workflow" grants only tools the chat called, with their observed access class.
- F33. The C14 bar holds on replay in CI and on the recorded live run.

### Inbox and attention (P1.6)

- F34. The inbox lists runs and pending approvals newest first, unread first among equals, with a count on the sidebar entry.
- F35. Approve from the inbox resumes the run; decline skips that write and the run continues; "Always allow" behaves as C9.
- F36. Opening a run clears its unread flag; the flag survives restart.
- F37. Each run status change notifies at most once: failures and approvals always, successes only with `notify.onSuccess`.
- F38. Running workflow runs appear in Mission Control and the dock's Agents pane.

## Epic acceptance, mapped

| Epic criterion (#845) | Proved by |
| --- | --- |
| No-project journey from a fresh install, UI only | E1, M1 |
| Project-bound code journey with worktree isolation | U-R12, E2, M2 |
| Window closed keeps schedules; one catch-up after sleep | F12, F15, U-T5, I-4, M3 |
| At most one run per occasion and one effect per write | F9, F11, F21, U-T1, U-T3, U-R7, E3 |
| Hostile connector text cannot use an ungranted tool | F18, U-R2, S-1 |
| Quota reserve holds workflows, never chats | F20, U-R5 |
| Scheduled tasks replaced, native schedules still editable | F25, F26, V-1, V-3 |
| `bun run build` and `bun run test` green | every PR |

## Unit Tests

Rust, in each module's `#[cfg(test)] mod tests`:

- `workflows.rs`
  - U-M1 `saves_and_reads_back_with_and_without_a_project` (F1)
  - U-M2 `an_edit_creates_a_new_immutable_revision` (F2)
  - U-M3 `widening_grants_clears_approval_and_narrowing_keeps_it` (F3)
  - U-M4 `a_no_project_home_is_created_once_and_removed_on_delete` (F4)
  - U-M5 `a_no_project_workflow_never_creates_a_project_row` (F4)
  - U-M6 `a_workflow_whose_project_folder_is_gone_is_disabled_with_a_reason` (F5)
  - U-M7 `each_invalid_definition_is_refused_with_its_reason`, table-driven over every F6 case
  - U-M8 `options_report_the_gate_reason_for_every_refused_adapter` (F7)
  - U-M9 `tool_class_fails_closed_to_write` (F7)
- `workflow_trigger.rs` (fake clock)
  - U-T1 `the_same_occasion_fired_twice_yields_one_run` (F9)
  - U-T2 `a_claim_during_an_active_run_is_skipped_or_returned` (F10)
  - U-T3 `an_expired_lease_is_reclaimed_and_the_stale_worker_cannot_commit` (F11)
  - U-T4 `spring_forward_and_fall_back_each_fire_once` (F12, C4)
  - U-T5 `three_missed_slots_yield_one_catch_up_with_missed_slots_three` (F12, C5)
  - U-T6 `paused_disabled_unapproved_and_removed_workflows_never_claim` (F13)
  - U-T7 `the_database_refuses_a_second_active_run` (partial unique index)
  - U-T10 `a_manual_dry_run_may_run_unapproved_and_a_schedule_may_not` (F13)
  - U-T11 `slots_before_the_anchor_are_never_caught_up` (F13a)
  - U-T8 `run_now_and_schedules_insert_runs_only_through_claim` (F8)
  - U-T9 `native_automations_are_never_dispatched` (F14)
- `workflow_run.rs` (fake adapter)
  - U-R1 `an_ungranted_tool_is_denied_with_its_name_in_the_reason` (F18)
  - U-R2 `hostile_connector_text_cannot_reach_a_shell_or_an_ungranted_tool` (F17, F18)
  - U-R3 `a_write_without_always_allow_raises_exactly_one_approval` (F18)
  - U-R4 `a_dry_run_executes_no_write_and_previews_each` (F19)
  - U-R5 `a_reached_reserve_waits_and_unknown_usage_runs_labelled` (C11)
  - U-R6 `chat_full_access_does_not_apply_to_workflow_runs` (C10)
  - U-R7 `a_resumed_run_refuses_a_write_that_already_executed` (F21)
  - U-R8 `teardown_declines_a_pending_ask` (F22)
  - U-R9 `result_block_maps_to_succeeded_failed_or_unverified` (C12)
  - U-R10 `a_user_turn_to_a_workflow_session_is_refused` (F24)
  - U-R11 `always_allow_records_and_accepts_atomically` (C9)
  - U-R12 `a_project_run_edits_only_its_own_worktree` (F16)
  - U-R13 `a_no_project_run_has_no_workspace_and_its_home_as_cwd` (F16)
  - U-R14 `a_budget_ceiling_stops_the_run_as_budget_exhausted` (F20)
  - U-R15 `usage_is_tagged_with_the_run_and_summed_onto_it` (F23)
- `workflow_draft.rs`
  - U-D1 `a_draft_is_never_persisted` (F30)
  - U-D2 `an_unconnected_app_becomes_a_connect_first_note` (F31)
  - U-D3 `save_as_workflow_grants_only_tools_the_chat_called` (F32)
- `workflow_inbox` (in `workflows.rs` or its own module)
  - U-I1 `unread_state_survives_restart` (F36)
- `bridge-protocol`
  - U-P1 every `workflows/*` params type round-trips and refuses unknown fields
  - U-P2 the existing drift and naming gates stay green (`tsgen`, `protocol_mirror`, params naming, registry and handler parity)

Sidecar, `node --test sidecar/claude-agent/test/workflow.mjs`:

- U-S1 the gate allows a read on a read-granted server, allows an always-allowed identity, asks for another tool on a write-granted server, denies anything else
- U-S2 the options never set `allowedTools`, set `strictMcpConfig`, and filter MCP servers to granted ones
- U-S3 a pending ask resolves to deny when the session tears down

Frontend, colocated vitest (`// @vitest-environment jsdom` where it mounts):

- V-1 `BridgeSidebar` shows "Workflows" and no "Scheduled tasks" (F25)
- V-2 `WorkflowsScreen` empty state, list row with "No project", disabled harness with its reason (F27, F28)
- V-3 the native schedules section renders an `AutomationCatalog` and keeps its actions (F26)
- V-4 the run view calls `sessions/fork_session` on "Continue in chat" (F29)
- V-5 the inbox orders, approves, declines and clears unread (F34 to F36)
- V-6 the draft form is prefilled and saves nothing until confirmed (F30)
- V-7 a running workflow run appears in Mission Control and the dock Agents pane (F38)

## Integration / Functional Tests

- I-1 Daemon round trip: `bridged` test client saves, approves and runs a manual workflow with the fake adapter, and receives `workflows/run_changed` for each status in order.
- I-2 Draft replay: 30 recorded drafting outputs in `testing/fixtures/workflow-drafts/` pass the validator and meet the C14 bar (F31 to F33).
- I-3 Notification de-duplication: one notification per run status change across a restart (F37).
- I-4 Host parity: the daemon host and the embedded host both start the workflow maintenance loop, and a schedule fires under the daemon with no desktop client attached (F15).

## Smoke Tests

- S-1 Boot with an existing data dir at schema 66: the migration lands, Workflows lists nothing, no maintenance error is logged.
- S-2 `bridge exec --json --method workflows/list_workflows --params '{}'` returns `{"workflows":[]}` on a fresh data dir.

## E2E Tests

In `src-tauri/bridged/tests/`, fake adapter, fake clock, temporary data dir:

- E1 No-project journey: draft from a recorded sentence, save, dry run, approve, schedule fires, write approval answered, result in the inbox, run openable and forkable.
- E2 Project journey on a real temporary Git repository: the run edits its own worktree; the original checkout's status is unchanged.
- E3 Restart mid-run: kill the daemon after one executed write, restart, and assert reclaim with no repeated write.

## Manual / CLI Tests

On a release build (not `tauri dev`), fresh data dir:

- M1 Workflows, New, type "every weekday at 9, summarise my Slack mentions". Check the draft's grants, Dry run, Approve, Run now, answer the write approval from the notification, open the result from the inbox, Continue in chat.
- M2 With a project open: a workflow with `files: worktree` that edits a file. Confirm a new worktree and an untouched checkout.
- M3 Schedule a workflow 15 minutes out, close the window, wait, reopen: one run. Sleep the Mac across two slots: one catch-up labelled as such.
- M4 Set the quota reserve to 90: the next run waits as `waiting_quota` while a chat still sends.
- M5 CLI checks:
  - `bridge exec --json --method workflows/workflow_options --params '{}'` lists Codex and OpenCode as refused for connector grants, with reasons.
  - `bridge exec --json --method workflows/run_workflow --params '{"workflowId":"<id>","requestId":"r1","dryRun":true}'` twice returns the same run, `created: false` the second time.
- Light and dark screenshots of each step are attached to #853.

## This PR

Ships this file only. Mockups of the Phase 1 screens, built from real components, are
attached to #846; they are not committed.
