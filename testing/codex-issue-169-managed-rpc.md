# codex/issue-169-managed-rpc — Test Contract

## Scope And Stated Assumptions

- A dedicated `agents` domain. The existing `marketplace` domain — plugins and
  connectors inside the harnesses' own plugin systems — is not touched, not
  renamed, and not overloaded. The two answer different questions: `marketplace`
  is about what runs *inside* an agent, `agents` is about whether the agent's
  runtime is installed at all.
- Two build gates make each method a full vertical, so a method is either complete
  or absent — there is no partial landing:
  - `MethodName::ALL` command names must equal the `generate_handler![...]` set
    exactly, in both directions.
  - Each Tauri command's argument names and JSON-relevant Rust types must match
    its contracted params struct field-for-field.
- Params structs use `deny_unknown_fields`, so every method contracted here
  rejects unknown top-level fields, per the 0.5 invariant #127 locked.
- Out of scope: the desktop UI (#170), any auth method, any credential storage,
  and any change to how the three integrations talk to their agents. No new agent.
- The RPC layer contains no ownership or installation logic of its own. It is a
  typed surface over #174's payload engine, #175's lifecycle coordinator, and
  #176's runtime resolution. A behaviour that is not already one of those is out
  of scope here.

## Functional Behavior

- `agents/list_managed_agents` returns one entry per built-in integration —
  Claude, Codex, OpenCode — carrying lifecycle state, whether a managed payload or
  an external runtime backs it, the resolved version, and bounded failure
  information. Never a bare boolean.
- `agents/inspect_managed_agent` returns a receipt summary for a managed payload
  (schema version, agent, version, platform, source, integrity, installation id,
  installed-at) and the detected external runtime path separately, so a caller can
  always tell which one it is looking at.
- `agents/install_managed_agent` starts an install and returns an operation id.
  `agents/repair_managed_agent` and `agents/uninstall_managed_agent` behave the
  same way for their operations.
- Readiness, running state, and bounded failure information are fields on the
  status payload rather than methods of their own, so one round trip answers "can
  this run" without a client stitching three responses together.
- Progress arrives as a transient notification carrying operation id, stage, and a
  terminal result. State changes arrive as a separate notification. Neither is
  durable: a client that reconnects refetches authoritative state from
  `list_managed_agents` rather than replaying progress.
- Errors are stable and specific, one code per condition the caller must be able
  to act on differently: unsupported platform, integrity failure,
  external-not-managed, busy or running, corrupt receipt, vendor prerequisite
  missing, and uninstall not permitted. A caller must never have to string-match
  an error message to distinguish them.
- An external runtime is visible through the read methods and cannot be removed
  through any of them: `uninstall_managed_agent` on an external agent fails with
  the external-not-managed code and touches nothing.
- Vendor authentication stays vendor-owned. A missing vendor login surfaces as the
  vendor-prerequisite code carrying the vendor's own message, and there is no
  method to submit, store, refresh, or clear a credential.

## Unit Tests

- `agents_domain_methods_are_registered_and_contracted` — every new method appears
  in `MethodName::ALL`, has a `generate_handler![...]` command, and its params
  fields match the command signature. Asserted through the existing gates rather
  than restated.
- `agents_params_reject_unknown_fields` — every params struct in the domain refuses
  an unknown top-level field.
- `agents_dtos_round_trip` — every DTO in the domain round-trips through JSON with
  camelCase wire names.
- `managed_agent_status_distinguishes_managed_external_and_absent` — the three
  backings produce three distinct payloads, and an external one carries no receipt.
- `uninstalling_an_external_runtime_is_refused_with_a_stable_code` — the error code
  is external-not-managed, distinct from every other code in the domain, and the
  runtime is untouched.
- `every_domain_error_condition_has_a_distinct_stable_code` — the seven conditions
  map to seven distinct codes with stable wire strings, and no two share one.
- `progress_notifications_are_transient_and_state_changes_are_separate` — the
  progress notification is declared transient, the state-change notification is
  its own name, and neither is durable.
- `vendor_prerequisite_errors_carry_the_vendor_message_verbatim` — a vendor login
  requirement surfaces under the vendor-prerequisite code with the vendor's own
  text unmodified, and no credential field exists anywhere in the domain.
- `no_method_in_the_domain_touches_credentials` — the domain declares no method
  whose name or params concern authentication, tokens, or credentials.

## Integration / Functional Tests

- The acceptance flow is drivable without the desktop app: from a fixture payload,
  `list → install → inspect → uninstall → list → install` again, asserting the
  state after each step and that no receipt survives the uninstall. Starting and
  stopping an agent is session lifecycle and already has methods; this domain owns
  the payload lifecycle.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` passes, including
  the method-registry and command-signature gates in the shell crate.
- Generated JSON Schemas and TypeScript artifacts exactly match the Rust contract
  — the generator is run and the tree is clean afterwards.
- `scripts/check-builtin-adapters.sh` remains green: Claude, Codex, and OpenCode
  keep passing #162.
- Existing `marketplace` methods are byte-identical in the regenerated artifacts,
  proving the plugin surface was not disturbed.
- `bun run build` and `bun run test` pass.

## Smoke Tests

- Drive install → start → stop → uninstall against a fixture payload over the
  daemon socket and confirm each response is typed and each notification arrives.
- Confirm an agent backed only by a PATH runtime lists as external, inspects
  without a receipt, and refuses uninstall.
- Disconnect mid-install and confirm the reconnecting client recovers
  authoritative state from `list_managed_agents` rather than from progress.

## E2E Tests

N/A for desktop E2E: the UI lands in #170. The flow above is the RPC-level
equivalent and is what this issue's acceptance asks for.

## Manual / cURL Tests

- N/A for cURL: the transport is a Unix-domain socket, not HTTP. The equivalent is
  the socket-level smoke flow above.
- Manually inspect a progress notification and a status response and confirm
  neither contains a credential, token, home path, or vendor configuration value.
