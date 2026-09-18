# feat/work-connector-health-and-reply — Test Contract

## Problem

Work's board reports "No recent activity to show" when a connector was never read at
all. The two states are indistinguishable to a user, so a Slack connector that is
disconnected, flapping, or needs re-authentication looks exactly like a quiet Slack.

### Confirmed root cause

1. `marketplace::sdk_connector_configs` filters the connector map to
   `health == Some(true)` before returning it. That filter is *correct* — handing a
   needs-auth connector to a `strictMcpConfig` run stalls the handshake — but it is
   the only place the connector appears.
2. `work_briefing_live::run` builds its scope from `configured.mcp_servers.keys()`,
   so an unhealthy connector is **absent**, not unhealthy.
3. `briefing_scope` therefore never yields it, so `record_source` is never called for
   it, so the run's coverage table has no row for it.
4. `workDashboard::toolsReadLine` reports only what is in `sources`, so the connector
   is invisible rather than reported as needing sign-in.
5. `work::board` only degrades on a run whose *status* is failed or cancelled. A run
   that succeeded having read nothing is `Ready`.
6. `WorkView` with zero items and `Ready` renders the generic empty state.

`ClaudeSdkConfiguration::connector_health` already carries the per-connector verdict
for **every** discovered connector, including the ones the filter drops. Nothing
currently reads it. That is the seam this change uses.

## Scope

Three deliverables, in dependency order:

1. **Health visibility** — a connector that was configured but could not be read is
   recorded on the run and surfaced, instead of vanishing.
2. **Mentions visibility** — an opt-in setting that includes already-read threads
   mentioning the user, so there is a deterministic signal to test against.
3. **Reply channel** — user-initiated, approval-gated replies to a Slack thread.

### Explicitly out of scope

Granting the unattended briefing worker write authority. The briefing reads untrusted
third-party text with nobody watching; `briefing_policy` is deny-by-construction for
exactly that reason. Bidirectionality is delivered as an attended, user-initiated
action instead. See §Reply channel.

---

## Functional Behavior

### 1. Health visibility

| Connector state at discovery | `mcp_servers` | `connector_health` | Expected source row |
|---|---|---|---|
| `✔ Connected` | present | `Some(true)` | `eligible` → then `consulted`/`succeeded`/`failed` from real tool results |
| `⚠ Needs authentication` | absent | `Some(false)` | `auth_required`, family resolved, detail names sign-in |
| `✗ Failed to connect` | absent | `Some(false)` | `auth_required`, family resolved, detail names sign-in |
| line carries no verdict | absent | `None` | `failed`, detail says the harness reported no verdict |
| user disabled the instance | either | either | **no row at all** — a deliberate exclusion is not a fault |

- Unreachable connectors are recorded on the run's coverage but **never** added to the
  briefing policy scope. They are not readable; the model is not offered them.
- The enabled-instances narrowing applies identically to reachable and unreachable
  sets. A connector the user switched off never produces a nag.
- Family resolution reuses `family_for_server` (whole-token match), so `unslacker`
  does not resolve as Slack.
- A connector whose family Bridge cannot resolve is still recorded, with family
  `unknown`.

### 2. Board degradation

- After a **succeeded** run, if any source row is `auth_required` or `failed`, the
  board's `suggestions.state` becomes `degraded` with a detail naming the affected
  families.
- A run that succeeded with every source `succeeded`/`consulted`/`eligible` stays
  `ready`.
- Board degradation never discards tasks. A degraded board with tasks still shows them.
- Existing degradation causes (run failed, run cancelled, unreadable settings row) keep
  their current precedence and detail text.

### 3. Empty-state honesty (UI)

- Zero items **and** at least one non-succeeded source → the empty state names the
  connector problem and offers the sign-in wording, not "No recent activity".
- Zero items **and** every source succeeded → the existing "No recent activity to
  show" copy is correct and is kept.
- Zero items and zero sources → "No tools were read." wording, not a caught-up claim.

### 4. Mentions visibility

- New setting `includeReadMentions: bool`, default `false`, `#[serde(default)]` so
  settings rows written before this change still deserialize under
  `deny_unknown_fields`.
- When `false`, the briefing task prompt is byte-identical to today's.
- When `true`, the prompt additionally asks for threads that mention the user within
  the window **even if already read**, and says read state must not be used to exclude
  an item.
- The setting changes prompt text only. It grants no tool, widens no scope, and
  changes no policy.

### 5. Reply channel — already present, corrected mid-implementation

Amended after reading the code. Two findings changed this section:

**The reply channel already exists and needs no work.** `connectors/connector_act`
takes `ConnectorActParams { item_key, action: Reply { text } | React { emoji },
approved: Option<bool> }`. Absent `approved` means "not decided": the host refuses and
returns the exact effect for a confirmation dialog, and approval is per call and never
sticky. `briefing_policy::compile_action_scoped` admits mutation tools only for an
`AuthorizedAction` that `connector_runs::authorize` will not produce until a human has
approved the literal text. That is the design this contract was about to specify, built
already. Nothing is added here.

**The surface with the reported symptom is the connector Inbox, not the Work board.**
`connectorSurface::emptyStateFor` already distinguishes degraded / waiting / caught-up,
so the Inbox was never lying about *coverage*. What it does is ask ingress for items
"received in the last 60 minutes and not yet read" — so a user who reads Slack in Slack
leaves ingress nothing to find, and the pane reports being caught up truthfully and
uselessly, forever.

So deliverable 3 becomes: `includeReadMentions` also swaps ingress from unread-only to
everything in the window.

- Off: both prompts are byte-identical to their current text.
- On: ingress asks for items whether or not they have been read, and the empty-answer
  rule stops naming unread so the two cannot disagree.
- Widening reads no extra tool and calls no extra tool.
- The announce-once ledger is untouched, so widening cannot replay an inbox: an item
  already announced is never announced again.
- Every safety clause in the ingress prompt survives both variants: read tools only,
  bodies copied verbatim, provider-owned identifiers, and message text treated as data
  rather than instructions.

Deliverables 1 and 2 are unchanged and still needed: the Work board had the same class
of defect the Inbox had already solved, and bringing it up to that standard is what
steps 1–3 do.

---

## Unit Tests

### Rust — `work_briefing_live`

- `unreachable_connectors_are_recorded_as_auth_required` — a connector in
  `connector_health` with `Some(false)` and absent from `mcp_servers` yields an
  `auth_required` source row.
- `unreachable_connector_without_verdict_is_recorded_as_failed` — `None` verdict
  yields `failed`, not `auth_required`.
- `disabled_instances_produce_no_unreachable_row` — narrowing excludes it entirely.
- `unreachable_connectors_never_enter_policy_scope` — the compiled scope contains only
  reachable servers.
- `unreachable_family_resolution_uses_whole_tokens` — `unslacker` does not resolve to
  Slack.
- `briefing_task_is_unchanged_when_mentions_are_off` — byte-equality against the
  current prompt.
- `briefing_task_asks_for_read_mentions_when_enabled` — prompt contains the mention
  clause and the "read state must not exclude" clause.

### Rust — `work::board`

- `succeeded_run_with_auth_required_source_degrades_board`
- `succeeded_run_with_all_sources_read_stays_ready`
- `degraded_board_still_returns_tasks`
- `run_failure_detail_takes_precedence_over_source_degradation`

### Rust — `work_connectors` / settings

- `work_settings_without_include_read_mentions_deserializes` — a stored row predating
  this change loads with the field defaulted to `false`.

### Frontend — `workDashboard.test.ts`

- `toolsReadLine` already covers auth/failed phrasing; extend with the zero-source case.
- `emptyStateCopy` (new pure helper) — returns connector-problem copy when any source
  is non-succeeded, caught-up copy only when everything succeeded.

### Frontend — `WorkView.test.tsx`

- `renders_connector_problem_instead_of_caught_up_when_a_source_needs_auth`
- `renders_caught_up_when_every_source_succeeded_and_there_are_no_items`
- `renders_degraded_banner_for_a_succeeded_run_with_an_unhealthy_source`

---

## Integration / Functional Tests

- Protocol artifacts regenerate cleanly and the drift test passes:
  `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-protocol`
- `WorkSettings` round-trips through the settings write/read path with the new field
  present and absent.
- A briefing run over a fixture harness reporting one healthy and one needs-auth
  connector produces: one readable scope entry, two coverage rows, a degraded board.

---

## Smoke Tests

- `bun run check` — clean (`tsc -b` + `cargo check --workspace`).
- `bun run test` — sidecar `node:test` + `vitest run` + `cargo test --workspace` green.
- `bun run build` — clean.

---

## E2E Tests

N/A for automated E2E — Bridge has no automated desktop E2E harness. Replaced by the
manual verification below, which is the path a reviewer actually runs.

---

## Manual / Verification Steps

Run against a real harness, since the whole defect is about real connector state.

1. **Reproduce the defect on `main`**
   ```bash
   claude mcp list | grep -i slack
   ```
   With Slack showing anything other than `✔ Connected`, open Work and refresh.
   Expected on `main` (the bug): "No recent activity to show", no mention of Slack.

2. **Verify the fix on this branch**
   Same starting state. Expected: the board reports Slack needs sign-in, shows the
   degraded banner, and does **not** claim you are caught up.

3. **Verify the healthy path still works**
   Reconnect Slack so `claude mcp list` shows `✔ Connected`, refresh.
   Expected: Slack appears in "Read …", board is `ready`.

4. **Verify mentions visibility**
   Enable *Include read mentions* in Settings → Work. Post a message in Slack that
   mentions you, read it, then refresh Work.
   Expected: the mention appears even though it is already read. With the setting off,
   a refresh does not surface it.

5. **Verify the Inbox actually polls** — the reported symptom
   With *Include read mentions* off, read every Slack mention in Slack, then refresh the
   Inbox. Expected: "You're all caught up" — correct, and indistinguishable from broken.
   Turn the setting on and refresh. Expected: mentions from the last hour arrive even
   though they are already read. That is the check for "is Slack even working".

6. **Verify the reply channel** (pre-existing; verifying, not adding)
   Reply to an inbox item. Expected: a confirmation naming the exact destination and the
   exact text, because `approved` is absent on the first call; on approval the message
   appears in Slack; on decline nothing is sent.

---

## Non-Goals / Invariants That Must Not Regress

- The `health == Some(true)` filter on what is handed to the provider stays. Unhealthy
  connectors must never be passed to a `strictMcpConfig` run.
- The briefing worker gains no write authority and no new tool.
- Evidence provenance is unchanged: citations still resolve only from Bridge-observed
  successful `tool_result`s.
- A brief that cites a failed, denied, or absent call is still refused.
- The append-only session forest and the policy engine keep sole ownership of their
  gates; nothing here grants a permission or widens a scope.
