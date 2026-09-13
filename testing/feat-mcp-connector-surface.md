# feat/mcp-connector-surface — Test Contract

Locked before implementation. Scope: issue #338 Phase 1 + Phase 2 — a reachable
vertical slice from "a DM arrives in Slack" to "you reply from Bridge's dock",
using **the harness's own authenticated MCP servers**. Bridge stays a manager:
no MCP client, no credential store, no Slack token, no OAuth.

## Design decisions locked here

1. **Templates, not raw HTML.** The issue's own open question proposes
   "templates first, raw HTML behind a flag". We take that. The harness emits a
   strict JSON *card* (`ConnectorCard`) validated against a schema; Bridge
   renders it with its own Tailwind components. No iframe in this slice, so
   there is no `postMessage` channel to harden and the card inherits Graphite &
   Paper tokens. A `ui://` raw-HTML iframe path stays possible later — the
   renderer is keyed on `card.kind`, so an `html` kind slots in beside it.
2. **Render only on notification.** No card is generated on a schedule, on open,
   or on refresh. A render run is issued for exactly one unread item and only
   after that item was observed as new. Reading the pane again reuses the stored
   card.
3. **Detection is free.** Connector availability comes from the health map
   Bridge already parses out of `claude mcp list`
   (`marketplace::claude_sdk_configuration`). No model turn to discover
   connectors.
4. **Every write is approval-gated and human-initiated.** No auto-reply. Inbound
   Slack text is untrusted data, fenced in prompts, never instructions.

## Functional Behavior

### Connector detection
- Given `claude mcp list` output where `claude.ai Slack` is `✔ Connected`,
  `connectors/list` reports family `slack` with `available: true`.
- Given `! Needs authentication`, the family reports `available: false` with
  reason `auth_required`, and the pane shows a sign-in hint, never an error.
- A family with no deterministic evidence resolver is `available: false` with
  reason `no_resolver` — it is never offered, per `work_connectors`.
- Detection performs **zero** model turns.

### Inbox ingress (polling)
- The poller asks the harness, on an interval, for unread mentions and DMs and
  receives a strict JSON delta. Cadence: 30s.
  *(Amended during implementation: the contract originally specified 30s focused
  / 180s unfocused. `bridge-core` has no window-focus signal to switch on — the
  GitHub poller declares the same pair and also only ever uses the focused one.
  Rather than ship a constant naming behaviour nothing implements, this slice
  has one cadence. Focus-aware backoff needs a focus signal in core first, and
  that is a separate change.)*
- An item is keyed by `(family, channel_id, message_ts)`. A key already seen is
  never re-announced, across restarts (the ledger is persisted).
- Items arriving while a poll is already in flight do not start a second poll
  (single-flight, as `GithubPoller::begin_refresh`).
- A poll that fails (harness down, quota, timeout) leaves the ledger untouched
  and surfaces a degraded status, not an empty inbox.
- Zero configured/available connectors ⇒ the poller does not run at all.

### Notification → card (the "only what's needed" rule)
- A newly observed item publishes `ConnectorItemArrived` exactly once.
- That event, and only that event, triggers **one** render run for **that one
  item**. Ten new items ⇒ ten cards, not one digest and not a re-render of the
  already-carded backlog.
- The render run returns a `ConnectorCard`; a card failing schema validation is
  rejected and the item falls back to a plain text card built by Bridge from the
  already-structured delta fields (never from model prose).
- `ConnectorCardReady` publishes when the card lands; the toast upgrades in
  place rather than being replaced.

### Reply / react (writes)
- `connectors/act` with kind `reply` or `react` requires an approval decision
  first. Calling it without one returns `ApprovalRequired` and performs no run.
- The approval payload states the literal effect — `Reply to @nina in #eng-alerts`
  plus the exact text to be sent.
- Denial records the denial and sends nothing.
- Approval launches one harness turn; success marks the item read and publishes
  `ConnectorItemResolved`.
- A reply whose target item is already resolved is refused (no double-send).

### Untrusted input
- Inbound message text reaching a prompt is wrapped in an untrusted-data fence.
- A message whose body contains injection-shaped instructions still produces a
  card; the text is displayed, never obeyed, and no action auto-fires.

## Unit Tests

**Rust — `connector_surface.rs`**
- `slack_connected_in_mcp_list_is_an_available_family`
- `needs_authentication_is_unavailable_with_auth_required`
- `a_family_without_a_resolver_is_never_offered`
- `detection_reads_only_the_health_map`

**Rust — `connector_inbox.rs`**
- `a_new_item_announces_once`
- `a_seen_key_is_never_reannounced`
- `the_seen_ledger_survives_a_restart`
- `a_failed_poll_preserves_the_ledger_and_reports_degraded`
**Rust — `connector_runs_live.rs`**
- `one_ingress_cycle_runs_per_family_at_a_time`
- `a_failed_cycle_still_releases_its_slot`
- `ingress_is_slower_than_the_github_poller_because_a_cycle_is_a_model_turn`
- `a_burst_of_arrivals_does_not_become_a_burst_of_model_turns`
- `resolving_an_item_marks_it_read`

**Rust — `connector_runs.rs`**
- `a_render_run_targets_exactly_one_item`
- `only_an_arrival_triggers_a_render`
- `a_malformed_card_falls_back_to_bridge_authored_text`
- `inbound_text_is_fenced_as_untrusted_data`
- `an_injection_shaped_body_still_renders_and_fires_nothing`
- `a_reply_without_approval_is_refused`
- `a_denied_reply_sends_nothing`
- `a_reply_to_a_resolved_item_is_refused`
- `the_approval_effect_names_the_target_and_the_text`

**Frontend — `connectorSurface.ts`**
- `toastKey` dedupes on `(family, channelId, messageTs)`
- `notificationText` renders DM vs mention vs thread-reply distinctly
- a card arriving upgrades a pending toast in place (same key)
- `unreadCount` counts unresolved items only
- relative time formatting is stable at boundaries

**Frontend — `ConnectorPane.test.tsx`** (jsdom)
- renders the skeleton state before a card arrives
- renders a card's blocks: header, thread context, suggested replies
- the reply box is disabled until a card is ready
- clicking Send opens Bridge's own approval confirm, not an immediate send
- denying closes the confirm and sends nothing
- an unavailable connector renders the sign-in hint, not an error
- honours `prefers-reduced-motion` (no entrance animation when set)

**Frontend — `ConnectorToasts.test.tsx`** (jsdom)
- a toast appears on arrival and deep-links into the dock pane
- dismiss removes only that toast
- toasts auto-expire after the TTL

## Integration / Functional Tests

- `bridged` dispatch: every new `connectors/*` method is routed and returns the
  contracted result shape.
- `protocol_mirror::result_payloads_mirror_core` passes for the new results.
- `tsgen::checked_in_artifacts_match_the_contract` passes — generated
  `protocol.ts` and `docs/protocol/schemas/` regenerated, never hand-edited.
- The registry↔Tauri-handler 1:1 test in `lib.rs` passes with the new commands.
- `messages::tests::params_types_are_named_after_their_method` passes.
- `bridge-deck every_command_signature_matches_its_contracted_params` passes.
- Event round-trip: `ConnectorItemArrived` reaches a subscribed client with its
  payload intact.

## Smoke Tests

- `bun run dev` with no Tauri: the dock shows an **Inbox** pane driven by mock
  connector data; a mock arrival fires a toast. No Rust required.
- `bun run check` green (tsc -b + cargo check --workspace).
- `bun run test` green (sidecar node:test + vitest + cargo test --workspace).
- Fresh-install reachability: from a clean launch with a connected Slack MCP,
  a user reaches a reply **using the UI only** — no CLI, no config file, no
  flag. (`end-to-end reachability gate`.)

## E2E Tests

- Live harness eval (`testing/evals/connector-render.md`): drive a real render
  run against the authenticated Slack MCP and assert the emitted card validates
  against the schema. Recorded as a replay fixture under `testing/fixtures/` so
  CI runs it without network or credentials.
- Full desktop E2E (`bun run tauri dev` → real Slack DM → toast → reply) is
  **manual**, documented below; it needs a live workspace and a second human.

## Manual Tests

```bash
# 1. Confirm the harness sees an authenticated connector (what Bridge parses)
claude mcp list | grep -i slack
#    expect: claude.ai Slack: https://mcp.slack.com/mcp - ✔ Connected

# 2. Mock-mode UI (no Tauri, no credentials)
bun run dev
#    open http://127.0.0.1:1420, open the dock, select Inbox

# 3. Targeted suites
bunx vitest run src/connectorSurface.test.ts
bunx vitest run src/components/ConnectorPane.test.tsx
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core connector

# 4. Schema drift
cargo run --manifest-path src-tauri/Cargo.toml -p bridge-protocol --bin generate-protocol-artifacts
git diff --exit-code src/protocol/generated/protocol.ts docs/protocol/schemas/
```

## Out of scope (explicitly deferred, Phase 3)

- Raw-HTML `ui://` iframe rendering — templates only in this slice.
- Push/webhook ingress into `bridged` — polling only.
- Linear / Notion / Gmail resolvers — the family enum already lists them; only
  Slack gets a resolver and a card template here.
- Cross-workspace Slack (Grid `team_id`) fan-out.
