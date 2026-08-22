# Work

A screen that answers one question: *what should I be doing right now?*

Bridge already knows a lot about work that is stuck — a worker blocked on a human, a failed
check, a branch that has drifted from its base. The connected tools know the rest: an unanswered
Slack thread, a review request, a customer mail, a Notion doc waiting on you. Neither half is
useful alone. Work joins them into one ranked list, refreshed in the background by a model turn
that runs in a harness you pin, and renders it as *data you act on* — never as instructions
anything but you can execute.

## The two classes of row

The screen's trustworthiness rests on keeping these visibly separate.

| | **Facts** | **Judgments** |
| --- | --- | --- |
| Source | Bridge's own SQLite | a model reading your connectors |
| Examples | worker `blocked_on_human`, failed `completion` check, `BaseBranchDivergence`, expired approval | "Reply to Priya's thread about the auth rollout", "PR #204 has been waiting three days" |
| Ranking | deterministic rules | model-assigned, with a stated reason |
| Availability | always, even offline | only when the pinned harness is logged in |
| Dismissible | no — it clears when the underlying state clears | yes |

Facts render above the fold under **Needs you** and never depend on a model call. Judgments
render under **Today** / **Later**. A judgment that *correlates* with a fact ("this Slack thread
is about the worker that is stalled in `bridge/auth`") is the highest-value row on the screen, so
the brief prompt is given the facts as context and can cite a `bridge:` source.

## Where the brief runs

Reuse the existing turn machinery — no new model client.

- **Harness:** pinned in Settings (a `briefing` purpose alongside the existing role profiles in
  `model_profiles.rs`). If the pinned harness is not installed or not logged in, the screen shows
  facts plus a plain banner naming what to fix, and links to Marketplace. No silent fallback.
- **Connector reach differs per harness, and this must be stated in the UI.** Bridge injects
  `mcpServers` only for Claude (`marketplace::claude_sdk_configuration()` →
  `claude_adapter.rs:95`). Codex and OpenCode see whatever their own config declares; Bridge can
  enumerate Codex and Claude connectors (`marketplace::catalog()` covers those two providers
  only) and cannot enumerate OpenCode's. So the header reports *available* connectors from
  Bridge's own auth states, and per-task evidence reports what was *actually read*. Bridge never
  claims a source was consulted on the model's word alone.
- **Session shape:** a new `sessions.kind = 'briefing'` row, workspace-less like a direct chat
  (`sessions.rs:119`), excluded from the sidebar tree and from Mission Control. It is a real
  session so replay, usage accounting, and failure surfacing all work for free — a briefing that
  goes wrong is inspectable like any other transcript.

### The permission profile is a new one

`writeMode: "ReadOnly"` cannot be reused. It resolves to
`allowedTools: ["Read","Grep","Glob","Bash"]` (`sidecar/claude-agent/options.mjs:3`), and because
`allowedTools` is an allowlist, that silently excludes every MCP tool — a read-only briefing would
see no Slack, no GitHub, no Gmail.

Add a `Briefing` mode that is **fail-closed on tool verbs**:

- local filesystem: `Read`, `Grep`, `Glob` — no `Bash`, no `Edit`/`Write`.
- MCP: an explicit allowlist of `mcp__<server>__<tool>` names built from a Bridge-maintained
  read-verb classification (`search`, `read`, `list`, `get`, `query`, `fetch`).
- anything unclassified is **denied**, and `permissionMode: "dontAsk"` so a background run fails
  rather than parking on an approval a human will never see.

This is load-bearing, not belt-and-braces: without it a briefing agent holds
`slack_send_message`, `notion-create-comment`, and Gmail send, and its input is text written by
other people. The allowlist is what makes the trust boundary real.

## The output contract

Copy the delegation pattern verbatim — it already solves this (`delegation.rs:479`).

A fenced ` ```bridge-work-brief ` block, one per turn, carrying `schemaVersion` (validated by the
same `validate_schema_version` rule), a `tasks` array capped at 12, and nothing else. Parse into
`ParseOutcome::{Parsed, Absent, Invalid}`; on `Invalid`, issue exactly one repair prompt via the
`ResultRepairTracker` equivalent; on a second failure record the run as `unstructured` with the
raw text kept for inspection and leave the previous brief on screen untouched. **A bad brief never
blanks the screen.**

Each task carries:

```
fingerprint      derived, not model-supplied (see below)
sourceKind       slack | github | gmail | notion | memory | bridge
externalId       thread ts, issue number, gmail thread id, session id …
title            imperative, ≤ 80 chars
why              one sentence, must reference the evidence
rank             1..n, model-assigned
projectId        null when it belongs to no repo
evidenceAt       timestamp of the newest evidence, from the source, not "now"
confidence       bps
```

A task with no resolvable `externalId` gets a fingerprint from its normalized title and is marked
`ephemeral`: it shows once and disappears on the next run instead of accumulating. Tasks whose
`why` does not cite evidence are dropped at parse time, not shown and argued with later.

## Reconciliation is the part that decides whether this is loved or muted

Every run upserts by fingerprint (`sha256(sourceKind + externalId)`).

- Preserve `firstSeenAt` and all user state (`pinned`, `snoozedUntil`, `done`, `dismissed`) across
  runs. Only `title`, `why`, `rank`, `evidenceAt`, `confidence` are rewritten.
- A task absent from a run is **not** deleted — it needs two consecutive misses before going
  `stale`, so one flaky connector cannot wipe the list.
- Dismissed and done fingerprints are fed back into the next prompt as a compact list, and
  re-proposals inside a 14-day cooldown are dropped during reconciliation unless `evidenceAt` is
  newer than the dismissal. The model is not trusted to remember; the store enforces it.
- `dismissedReason` (free text, optional) is included in that feedback so the next brief can learn
  "I don't own deploys" without a settings page for it.

## Scheduling

Mirror `learning_job.rs`, which already solved this shape — including the part where the schedule
lives *outside* Bridge.

- **In-app cadence:** `cadence_minutes`, `next_run_at`, `enabled`, `run_budget_microusd`,
  `run_budget_tokens`, `mode` — the `LearningSchedule` field set (`learning_job.rs:127`). Plus a
  run on app focus when the last brief is older than the cadence.
- **Lease + idempotency:** a 15-minute lease and an idempotency key per run, so a focus-triggered
  run and a scheduled run cannot double-charge (`LEASE_MINUTES`, `learning_job.rs:11`).
- **External wake-up:** `bridge work brief --database <db> --trigger claude:ID` alongside the
  existing `bridge learning run` (`bridge-client/src/bin/bridge.rs:32`), with the same
  registration/expiry/credential-ref checks and the same wake-up-only prompt discipline
  (`docs/prompts/codex-learning-scheduled-task.md`). Bridge does not create, enumerate, or repair a
  Claude/Codex/OpenCode schedule — it hands you the command and the registration id.
- **Cost is visible:** the briefing turn writes to `usage_ledger` with its own source, so the
  Usage widget shows what background work costs. A background feature that quietly spends a
  subscription is a feature people turn off.

## Protocol surface

New `work/` domain in the `MethodName` registry (`bridge-protocol/src/methods.rs`), params/result
schemas under `docs/protocol/schemas/`, then regenerate:

```bash
cargo run --manifest-path src-tauri/Cargo.toml -p bridge-protocol --bin generate-protocol-artifacts
```

| Method | Result |
| --- | --- |
| `work/get_work_board` | `WorkBoard` — facts, tasks, last run, schedule, connector availability |
| `work/refresh_work_brief` | `WorkBriefRun` (accepted / duplicate / budget-exceeded / disabled) |
| `work/resolve_work_task` | `WorkBoard` — pin, snooze, done, dismiss |
| `work/start_work_task` | `BridgeState` — creates the session, returns its id |
| `work/update_work_schedule` | `WorkSchedule` |
| `work/register_work_trigger` | `WorkTriggerRegistration` |

`get_work_board` reads the store only and never blocks on a model — the screen opens instantly with
the last brief and refreshes underneath.

## Storage — migration 23

Following `store.rs` (currently at 22):

- `work_briefs` — run id, trigger kind, harness, model, status, idempotency key, lease, budget
  spent, source counts, parse disposition, raw text on failure.
- `work_tasks` — fingerprint (pk), the contracted fields, `firstSeenAt`, `lastSeenAt`,
  `missCount`, user state, `dismissedReason`, `briefId` of the run that last touched it.
- `work_triggers` — external registrations, same shape as the learning ones.
- `work_schedule` — single row, `LearningSchedule`-shaped.

## The screen

Graphite & Paper, and this screen is where the "colour only carries meaning" rule earns its keep:
a prioritized list is exactly the thing that turns into a wall of red badges. **Priority is carried
by order and a mono rank, not by hue.** Status ink appears only on facts, reusing the tone
vocabulary from `workerStatus.ts` so a Work row and its Mission Control tile read the same.

- Sidebar rail gets a third destination next to Marketplace and Settings
  (`BridgeSidebar.tsx:488`), and `App.tsx` a `view === "work"` branch — a lazy screen like the
  other two.
- **Header strip:** last briefed relative time · harness + model used · connector chips (mono,
  muted; unreachable ones struck through) · Refresh. Nothing else.
- **Needs you:** facts, deterministic order, each with the one action that clears it (resolve
  approval, open the worker, refresh base).
- **Today:** ranked judgments. One row = rank, title, `why` in muted text, source chip, age, and a
  right-aligned `Start`. Row click opens the evidence — the permalink, the issue, the thread —
  never a rendered version of it.
- **Later:** snoozed and low-confidence, collapsed, count in the header.
- **Empty and degraded states are first-class, not afterthoughts:** no harness pinned; harness
  pinned but not logged in; no connectors; brief failed to parse; budget exhausted; every source
  reachable but genuinely nothing to do. That last one should feel like a reward, not a bug.
- 420px works: the rank and action stay, the source chip and age fold under the title.

Actions are all human-initiated. `Start` opens a normal Bridge session prefilled with the task
title, `why`, and a link to the evidence — the task text enters as *quoted material attributed to
its source*, never as the system prompt.

## Steps

Each step is a PR with its test committed first, matching the sidebar redesign's cadence.

1. **Contract** — `work/` methods in the registry, JSON schemas, generated TS, migration 23. Test:
   the method registry round-trips and the migration is idempotent.
2. **Facts** — derive the `Needs you` list from existing state (human-blocked queue, completion
   checks, divergence, expired approvals). Test: each fact appears exactly once and clears when
   its underlying state clears.
3. **Screen** — the rail destination and the board rendering facts, with every empty and degraded
   state. Test: degraded states render without a brief present.
4. **Briefing profile** — the `Briefing` write mode in the sidecar with the fail-closed MCP
   allowlist. Test: an unclassified `mcp__*` tool is denied; a mutating verb is denied even when
   its server is connected.
5. **The brief** — headless briefing session, prompt in `docs/prompts/work-brief.md`, the
   `bridge-work-brief` parser with one repair. Test: absent / malformed / wrong-schema-version /
   evidence-free tasks each behave as specified, and a failed brief leaves the prior board intact.
6. **Reconciliation** — fingerprint upsert, miss counting, dismissal cooldown, feedback list.
   Test: a dismissed task does not return within the cooldown; a single missing run does not
   delete a task; user state survives a rewrite.
7. **Schedule** — cadence, lease, budget, focus trigger, `bridge work brief` CLI and external
   registration. Test: concurrent runs collapse to one; an over-budget run is refused, not
   truncated.
8. **Actions** — start / snooze / done / dismiss, quoted-evidence prompt construction, usage
   attribution.

## Risks worth deciding on early

- **Prompt injection is the headline risk.** Every judgment on this screen was written by a model
  that read text authored by other people. The mitigations are the fail-closed tool allowlist, the
  read-only filesystem, tasks-as-data with no auto-dispatch, evidence rendered as links rather
  than content, and `secret_interception.rs` on the way out. This should be stated in the doc that
  ships with the feature, not just in code comments.
- **A brief that is 70% right is worse than no brief.** Hence the evidence requirement, the
  dismissal cooldown, and keeping facts separate — a user who dismisses three bad rows in a row
  will never trust the fourth good one.
- **Cost drift.** Cadence × connector count × context size is a real subscription line item. Budget
  fields exist from step 7; default the cadence conservatively (hourly, and only while the app is
  focused) and let people tighten it.
- **OpenCode is a second-class citizen here** and the UI should not pretend otherwise: Bridge
  cannot enumerate its connectors, so a pinned OpenCode harness gets "connectors unknown" in the
  header rather than a confident chip list.
