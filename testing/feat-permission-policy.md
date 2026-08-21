# feat/permission-policy — Test Contract

Slice 1 of #255: the policy layer, the universal bypass switch, and the audit
trail. Parts 1 and 4 of the issue.

**Explicitly out of scope, by agreement**, each getting its own PR:
- Part 2 — inline approve/decline in the parent chat, which needs the mirrored
  `delegation.blocked` event to carry the child approval's durable `eventId`.
- Part 3 — graduated modes: auto-allow workers, inherited grants, command-pattern
  allowlists, and the global → workspace → agent-class scope hierarchy.

The slicing is not about size. The two gates that must survive bypass are a
security boundary, and a diff that grants permission automatically deserves to be
read on its own.

## What the seam actually is

Worth stating because it determines how narrow this can be. All three providers
raise approvals over their own control channel, and `agent.rs` normalizes each to
one `approval.requested` event:

| provider | frame |
|---|---|
| Codex | `*/requestApproval`, `item/tool/requestUserInput`, `mcpServer/elicitation/request` |
| Claude | `control_request` subtype `permission` / `can_use_tool` |
| OpenCode | `permission.v2.asked` / `permission.asked` |

So there is exactly **one** provider-agnostic place to apply policy, and it is the
same place a human decision is applied. Bypass reuses `resolve_approval` rather
than growing a second decision path — the thing most likely to drift into granting
something the human path would have refused.

Adapter-flag mapping (Claude's `permissionMode`, Codex's `sandbox_policy`) is
deliberately **not** in this slice. It is an optimization that saves a round trip;
the control-channel seam is authoritative and universal, and mapping flags per
adapter is where a provider-specific mistake would hide. Noted in the issue as a
follow-up.

## Functional Behavior

### 1. The policy is durable, Rust-owned, and read in one place

- A `PermissionPolicy` lives in the config domain, stored in
  `configuration_entries` under `kind='permission_policy'` like every other
  configured thing, and is returned as part of `ConfigState` so reading it needs
  no new method.
- Exactly one new wire method: `config/save_permission_policy`, params
  `{policy}`, returning the whole `ConfigState` — the shape every other config
  mutation already uses.
- Slice-1 shape, with room for slice 3 to add fields without a breaking change
  (every added field must be `#[serde(default)]`):
  ```
  PermissionPolicy { bypassAll: bool, updatedAt: String }
  ```
- Default is `bypassAll: false`. A fresh install asks, as today.

### 2. Bypass on means no agent asks — with two exceptions, stated in the UI

- With `bypassAll: true`, an `approval.requested` from any provider is resolved
  `accept` immediately, through `resolve_approval` — the same function a human
  click calls, so the `approval.resolved` event, the worker lifecycle transition,
  the session/workspace status updates, and the parent's mirrored card all behave
  exactly as they do for a human decision.
- Two gates survive, and the toggle's own copy says so:
  - **worker write-scope** (`delegation_path_scope`) — authorization, not
    convenience. It is raised by `policy.rs` as a typed forest entry, never over a
    provider control channel, so it structurally cannot reach this seam. The
    auto-path *additionally* refuses it by name, and a test proves the refusal
    rather than trusting the structure.
  - **browser outward effects** (send/purchase/publish/credential) — raised in
    `browser_bridge.rs` against its own `pending_approval` with its own audit
    channel, never as a session `approval.requested`. Asserted, not assumed.
- Auto-approval never fires on an unpersisted event. `store::session_event` can
  return `sequence: 0` for a frame it did not persist, and resolving sequence 0
  would answer the wrong approval.
- Auto-approval is deferred past the correctness lock, like every other
  post-persist action in `handle_agent_value` (`pending_child_approval`,
  `pending_peek`, `pending_steer`), because reaching the runtime needs the adapter
  map.

### 3. Loud while on

- Every auto-approval writes a reason-ledger row naming the policy that matched:
  `approval.auto_allowed`, body naming `bypass_all`. One row per approval, so the
  Permissions settings page can list recent auto-approvals from durable state
  rather than from memory.
- The conversation shows an `approval.resolved` item with the decision, exactly as
  a human resolution does — no silent grants.
- A persistent badge in the app chrome reads "Approvals bypassed" whenever the
  policy is on, so the mode is never invisible.

### 4. Settings surface

- A **Permissions** section in the settings screen: the bypass toggle, copy naming
  the two surviving gates, and a list of recent auto-approvals read from the
  reason ledger.
- The toggle is optimistic-free: it saves through `save_permission_policy` and
  renders from the returned `ConfigState`, so what is shown is what is stored.

## Unit Tests

### Rust — `bridge-core`

`agent_config.rs`
- `permission_policy_defaults_to_asking` — a fresh database returns
  `bypassAll: false`; nothing is auto-approved until someone opts in.
- `permission_policy_round_trips_through_the_config_store` — save, re-read via
  `state()`, and the value survives; `updatedAt` is stamped.
- `saving_a_policy_leaves_the_rest_of_config_state_alone` — harnesses, agents, and
  the default agent id are untouched.

`live_turn.rs` — a new `permission_policy_tests` module
- `an_approval_is_auto_accepted_when_bypass_is_on` — a live session raises
  `approval.requested`; the provider receives `accept` through `respond`, an
  `approval.resolved` entry exists, and an `approval.auto_allowed` ledger row
  names `bypass_all`.
- `an_approval_waits_for_a_human_when_bypass_is_off` — nothing is responded, the
  session goes `waiting`, and no ledger row is written.
- `a_write_scope_approval_is_never_auto_accepted` — a `delegation_path_scope`
  approval with bypass on is left pending: no `respond`, no ledger row, still
  waiting on a human.
- `auto_approval_never_answers_an_unpersisted_approval` — a frame that persists as
  sequence 0 is not resolved.
- `a_worker_approval_auto_accepted_leaves_the_worker_running_not_waiting` — the
  lifecycle transition matches the human path, so the approval deadline and the
  parent's mirrored card do not disagree.
- `bypass_does_not_touch_the_browser_gate` — the browser bridge's own pending
  approval is unaffected by the policy.

`protocol_mirror.rs`
- the existing `result_payloads_mirror_core` gate must cover the new
  `permission_policy` field on `ConfigState`; if it does not fail when the two
  structs disagree, that is a finding about the gate, recorded either way.

### TypeScript — vitest

`src/components/SettingsScreen.test.tsx`
- `offers a permissions section with the bypass switch off by default`.
- `names the two gates that survive bypass` — the copy must state them, because
  the issue makes that copy part of the contract.
- `lists recent auto-approvals` from a supplied ledger.

`src/App.test.tsx` or the chrome component's own test
- `shows a persistent badge while approvals are bypassed`, and no badge when off.

## Integration / Functional Tests

- `cargo test --workspace` green, including the protocol registry↔handler 1:1
  test and `tsgen::checked_in_artifacts_match_the_contract` — the new method must
  be regenerated into `src/protocol/generated/protocol.ts` and
  `docs/protocol/schemas/`, run from **this** checkout's root.
- `bun run test`, `bun run build`, `bun run check` green.

## Smoke Tests

- Protocol checklist walked explicitly: `methods.rs` triple,
  `messages/config.rs` params with `deny_unknown_fields`, `messages/mod.rs` typed
  table, `bridged/src/dispatch.rs` arm, `bridge-core/api.rs` fn, `lib.rs` Tauri
  command plus `generate_handler![]`, then regenerate artifacts.
- `cargo clippy -p bridge-core --all-targets` — no new warnings versus
  `origin/main` in any touched file, measured rather than asserted.

## E2E Tests

N/A as automated. Manual path, in a release bundle:

1. Settings → Permissions, flip **Bypass all approvals**. Badge appears.
2. Start a Codex chat and ask it to run a command that would normally prompt.
   No prompt appears; the conversation shows the resolution; Permissions lists it.
3. Delegate a write-capable worker. The write-scope approval card **still**
   appears — that is the gate surviving, and it is the single most important thing
   to see with one's own eyes in this slice.
4. Turn bypass off. The next command prompts again.

## Manual / cURL Tests

No HTTP surface. Against a live daemon:

```
./src-tauri/target/debug/bridge exec --json \
  --data-dir "$HOME/Library/Application Support/dev.bridge.deck" \
  --method config/save_permission_policy \
  --params '{"policy":{"bypassAll":true,"updatedAt":""}}'
```

then `config/get_config_state` must show `permissionPolicy.bypassAll: true`, and

```
SELECT kind,body,created_at FROM events WHERE kind='approval.auto_allowed' ORDER BY id DESC LIMIT 5;
```

must gain one row per auto-approved request.
