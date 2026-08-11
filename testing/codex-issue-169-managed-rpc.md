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
  Claude, Codex, OpenCode — carrying state, which copy would actually launch
  (`managed`, `external`, `explicit`, `bundled`, or none), the resolved executable
  path, and the version. Never a bare boolean.
- Ownership and launchability are separate fields, because they are separate
  questions: Bridge owns a drifted payload — it is `removable` — but would not
  launch it, so `backing` describes the copy that would run while `removable`
  describes what Bridge owns.
- `agents/inspect_managed_agent` returns a receipt summary for a managed payload
  (schema version, agent, version, platform, source, integrity, installation id,
  installed-at) and the detected external runtime path separately, so a caller can
  always tell which one it is looking at.
- `agents/install_managed_agent`, `agents/repair_managed_agent`, and
  `agents/uninstall_managed_agent` run to completion before returning, and their
  result says what happened — installed, repaired, removed, already current,
  already absent — plus the post-operation status, so a client needs no second
  round trip. There is deliberately no operation id: handing back a correlation id
  for a job no stream will ever reference would invite a client to wait forever.
  Backgrounding these is a future change with a new result shape, not a
  reinterpretation of this one.
- Readiness, running state, and bounded failure information are fields on the
  status payload rather than methods of their own, so one round trip answers "can
  this run" without a client stitching three responses together.
- A state change arrives as one transient notification carrying the agent id only.
  It is a refetch hint like its neighbours, so it *is* replayed to a subscriber
  that fell behind — otherwise a lagging client never learns it must re-read — and
  authoritative state always comes from `list_managed_agents`. There is no
  progress notification, because the operations complete before their method
  returns.
- Errors are stable and specific, one code per condition the caller must be able
  to act on differently: unsupported platform, integrity failure,
  external-not-managed, busy, corrupt receipt, vendor prerequisite missing,
  uninstall not permitted, and unknown agent. A caller must never have to
  string-match an error message to distinguish them, and both host modes deliver
  the same `{code,kind,message}` envelope — a code dropped in one mode would push
  clients straight back to text matching.
- Removal refuses while a provider process is still alive for that agent, under
  the busy code. Deleting a payload out from under a running session is the
  failure this epic exists to prevent, so it is checked against tracked adapter
  pids and their recorded OS identity, and a stale row must not make an agent
  permanently un-removable.
- A payload-engine failure keeps its own condition: a corrupt receipt reports as
  corrupt, an integrity failure as integrity, and anything else keeps its
  1000-range code rather than being flattened into "uninstall not permitted".
- An external runtime is visible through the read methods and cannot be removed
  through any of them: `uninstall_managed_agent` on an external agent fails with
  the external-not-managed code and touches nothing.
- Vendor authentication stays vendor-owned. A missing vendor login surfaces as the
  vendor-prerequisite code carrying the vendor's own message, and there is no
  method to submit, store, refresh, or clear a credential.

## Unit Tests

- `agents_dtos_round_trip` — every DTO in the domain round-trips with camelCase
  wire names.
- `agents_params_reject_unknown_fields` — every params struct in the domain, not
  just one, refuses an unknown top-level field.
- `only_a_managed_backing_is_removable` — the API answers removability so a client
  never infers it from a state string.
- `an_operation_result_says_what_happened_not_that_something_started` — the result
  carries no operation id and does carry the post-operation status.
- `no_field_in_the_domain_concerns_credentials` — the domain's wire surface
  contains no credential-shaped field.
- `every_domain_condition_has_a_distinct_stable_code` — the conditions map to
  distinct codes inside the managed-agent range.
- `an_underlying_failure_keeps_its_own_code` — a wrapped `BridgeError` keeps its
  1000-range code instead of being flattened.
- `store_failures_keep_their_own_condition` — corrupt-receipt, integrity, and I/O
  failures each report as themselves on the destructive path, and the real cause
  stays reachable through `source()`.
- `serialized_errors_carry_their_code` — the error payload carries `code` and
  `kind`, so a client branches on code rather than message text.
- `a_live_provider_process_blocks_removal` — a live tracked process refuses removal
  under the busy code, a different agent is unaffected, and a stale row does not
  make an agent permanently un-removable.
- `a_vendor_prerequisite_carries_the_vendor_message_verbatim` — the vendor's own
  text survives unmodified.
- `the_built_in_agent_list_matches_the_recipes` — the agent list and the recipes
  cannot drift into disagreeing about which agents exist.
- `an_unknown_agent_is_refused_before_any_storage_access` — refused on identity
  alone, under its own code, before touching the filesystem.

## Integration / Functional Tests

- The acceptance flow is drivable without the desktop app. Install and repair
  fetch from the vendor, so the network-touching half is exercised by hand rather
  than in CI, consistent with the crate's other live tests; the read path,
  removal, refusals, and error codes are covered by unit tests above.
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

- Drive install → inspect → uninstall against a fixture payload over the daemon
  socket and confirm each response is typed and each notification arrives.
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
