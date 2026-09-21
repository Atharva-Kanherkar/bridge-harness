# feat/connector-inbox-read-mentions — Test Contract

## Problem

The connector Inbox — the pane headed `Slack · via your harness` — sits on
**"You're all caught up · Nothing new since the last check"** indefinitely, and there
is no way to tell that apart from a Slack connector that is not working at all.

### Confirmed root cause

`connector_runs::ingress_prompt` asks the harness for items

> received in the last `INGRESS_LOOKBACK_MINUTES` (60) minutes **and not yet read**

Anyone who reads Slack in Slack has nothing unread by the time the next 30-second cycle
runs, so ingress correctly returns `{"items":[]}` forever. The pane is not lying:
`connectorSurface::emptyStateFor` already separates `degraded` / `waiting` /
`caught-up`, and this is a genuine `caught-up`. It is simply unfalsifiable — a working
connector and a broken one produce the same empty pane, so "is Slack even working?"
cannot be answered from the surface that is supposed to answer it.

### What is already built and is not in scope

- **The reply channel.** `connectors/connector_act` carries
  `Reply { text } | React { emoji }` with a per-call `approved: Option<bool>`. Absent
  means "not decided": the host refuses and returns the exact effect, and `ConnectorPane`
  renders *that* sentence as the confirmation rather than composing its own. Approval is
  never sticky. `briefing_policy::compile_action_scoped` admits a mutation tool only for
  an `AuthorizedAction` that `connector_runs::authorize` will not produce until a human
  approves the literal text. Bidirectional communication works today; nothing is added.
- **Health honesty.** `emptyStateFor` already distinguishes an inbox that could not be
  read from one that is empty, and `poll_claimed` already records a poll failure when no
  harness has the family connected.

## Scope

One deliverable: an inbox-owned setting that stops read state excluding an item, so the
user can produce a signal on demand and confirm the pipe end to end.

Everything lives in the connector surface. The Work board is explicitly untouched.

---

## Functional Behavior

### 1. The setting

- Stored by the connector surface in `configuration_entries` under
  `kind = "connectors"`, `id = "settings"`. It is not part of `WorkSettings` and does not
  change Work's behaviour.
- Defaults to `false`. An absent row, an unreadable row, or a malformed payload all read
  as `false` rather than failing the inbox — the inbox is useful without it.
- Round-trips: writing `true` then reading returns `true`; rewriting is an upsert, not a
  second row.

### 2. Ingress

- With the setting **off**, `ingress_prompt` is byte-identical to its current text.
- With it **on**, two clauses change and nothing else:
  - the window clause stops saying `and not yet read` and asks for items whether or not
    they have been read;
  - the empty-answer rule stops saying `Nothing is unread?`, so the instruction and the
    rule cannot disagree about what an empty list means.
- Widening asks for **no extra tool** and **no extra permission**. The same read tools
  answer both variants; the policy scope is unchanged.
- Every safety clause survives both variants: read tools only, bodies copied verbatim,
  provider-owned identifiers copied exactly, and message text treated as data rather
  than as instructions.
- The timer-driven poll and the pane's manual refresh both honour the stored setting —
  there is no path that reads one and not the other.

### 3. The announce-once invariant

- Turning the setting on must not replay an inbox. `connector_inbox` announces an item
  if and only if inserting its key succeeded, so an item already announced stays
  announced and is not re-notified.
- Resolved items (replied, reacted, dismissed) are never re-announced when the window
  widens to include them.

### 4. Surface

- The pane exposes the toggle itself — this is an inbox setting and there is no
  connector settings page to hide it in.
- The toggle reflects stored state on load, persists on change, and triggers a refresh
  so the effect is immediate rather than up to 30 seconds later.
- The control is labelled and reachable by assistive technology.

---

## Unit Tests

### Rust — `connector_runs`

- `the_ingress_prompt_reaches_read_items_only_when_asked` — off contains
  `and not yet read` and `Nothing is unread?`; on contains neither and asks for items
  regardless of read state.
- `widening_past_read_state_changes_nothing_else_about_ingress` — reconstructs the off
  variant from the on variant by reversing exactly the two clauses, and pins every
  safety clause in both.

### Rust — `connector_settings`

- `connector_settings_default_to_off_when_absent`
- `connector_settings_round_trip_and_a_rewrite_is_an_upsert`
- `an_unreadable_connector_settings_row_reads_as_off` — a malformed payload must not
  fail the inbox.

### Frontend — `ConnectorPane.test.tsx`

- `renders_the_read_mentions_toggle_from_stored_state`
- `persists_the_toggle_and_refreshes_so_the_change_is_visible_now`

---

## Integration / Functional Tests

- Protocol artifacts regenerate cleanly and the drift test passes:
  `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-protocol`
- The new method is registered exactly once and has exactly one handler (the existing
  registry↔handler pin covers this).
- `ConnectorInboxResult` carries the setting so the pane renders stored state without a
  second round-trip.

---

## Smoke Tests

- `bun run check` — clean.
- `bun run test` — green.
- `bun run build` — clean.

---

## E2E Tests

N/A — Bridge has no automated desktop E2E harness. Replaced by the manual verification
below, which is the check the user actually asked for.

---

## Manual / Verification Steps

1. **Reproduce**
   Read every Slack mention in Slack. Refresh the Inbox.
   Expected: "You're all caught up · Nothing new since the last check" — correct, and
   indistinguishable from a broken connector. This is the complaint.

2. **Verify the fix**
   Turn the toggle on in the Inbox header. Refresh.
   Expected: mentions and DMs from the last 60 minutes arrive **even though they are
   already read**. This is the answer to "is Slack even working".

3. **Verify it does not replay**
   Refresh again with the setting still on.
   Expected: nothing re-announces. Items already announced stay put.

4. **Verify the reply path still works** (pre-existing)
   Reply to an item. Expected: a confirmation naming the exact destination and the exact
   text; on approval it appears in Slack; on decline nothing is sent.

5. **Verify off is unchanged**
   Turn it off, read everything in Slack, refresh.
   Expected: caught-up, exactly as before this change.

---

## Non-Goals / Invariants That Must Not Regress

- The Work board, `WorkSettings`, and the briefing run are untouched by this branch.
- The briefing worker gains no write authority and no new tool.
- Ingress still forms no judgements: no summary, no priority, no suggested action.
- The announce-once ledger keeps sole ownership of what counts as new.
- `INGRESS_LOOKBACK_MINUTES` is unchanged — this widens *read state*, not the window.
