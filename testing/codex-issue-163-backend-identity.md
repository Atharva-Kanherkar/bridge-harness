# codex/issue-163-backend-identity — Test Contract

Locks #163, the P0 of the marketplace epic #171: separate the public agent
identity from the backend that implements it, and make every session say which
backend and version actually served it.

## Scope And Stated Assumptions

- **Four identities, not one string.** Today one value answers four questions:
  `sessions.harness` is simultaneously the public agent name, the adapter
  registry key, the implied transport, and — through `binary::resolve` — the
  runtime that gets launched. This splits them:
  - `AgentId` — the public identity, what a user picks and what history is filed
    under.
  - `BackendId` — which implementation served it (`claude.agent-sdk`,
    `codex.app-server`, `opencode.server`).
  - `BackendVersion` — the concrete version of that implementation.
  - `InstallationId` — the managed payload a backend was launched from, when
    Bridge owns one.
- **`AgentId` is `HarnessId`, deliberately.** #160 already opened the wire
  identity and removed the `acp:<id>` namespace, and its doc comment already
  states the rule this issue asks for — one id per agent, never per integration.
  So the ACP-specific namespace generalization #163 asks for is *already landed*;
  what is missing is the other half, the backend provenance it promised would be
  "recorded separately". `AgentId` is introduced as an alias of `HarnessId`
  rather than a rename: renaming the wire type would churn every message for a
  spelling change and break protocol 0.9 compatibility for no behavioural gain.
  The separation that matters — agent versus backend — is enforced by
  `BackendId` being a genuinely distinct type that cannot be passed where an
  `AgentId` is expected.
- **`ManagedPayloadReceipt.installation_id` keeps its `String` type.** The
  receipt is a persisted, digest-validated artifact whose corrupt-receipt paths
  are covered by #174's suite. `InstallationId` is defined here and applied at
  the binding boundary, parsing the receipt's value on the way in. Retyping the
  receipt field is a #164 concern, not a P0 risk worth taking.
- **A backend change is refused; a version change is recorded, not refused.**
  #163 requires "explicit, separately-authorized continuation if a session ever
  changes backend," and that a resume "never silently chooses a different
  backend/version." Those are read as different consequences on purpose:
  refusing on every version difference would break resume on every ordinary
  vendor update, which is a routine event — Claude moved 0.3.209 forward twice
  during #161 alone. So a differing `BackendVersion` resumes and records a
  durable `backend.version_changed` fact, and a change of *backend* happens only
  through an authorization. Neither is silent.

  (The first half of this assumption — that a backend differing from a fresh
  resolution blocks the resume — was amended once the launch paths were wired.
  See "A bound session sticks to its backend" below, which supersedes it.)
- **No new RPC method, and no UI.** The authorization is a `bridge_core::api`
  entry point. Its wire and desktop surface belong with #166's control plane;
  adding a method here would put a user-facing control in front of a resolver
  that still has exactly one candidate per agent. Recorded so its absence reads
  as a decision.
- **The three built-ins are frozen, not re-described.** `BuiltInAgentContract`
  gains no field, so `testing/fixtures/builtin-compatibility-report-v1.json`
  stays byte-identical and the #162 gate keeps its meaning. The backend table is
  a separate structure, tied to the frozen contracts by a test that fails if the
  two ever disagree.
- Out of scope: the compatibility catalog (#164), the integration framework and
  its drivers (#166), verification (#168), any second backend candidate for a
  real agent, and any new agent.

## Functional Behavior

- `BackendId`, `BackendVersion`, and `InstallationId` are validated newtypes.
  `BackendId` shares `HarnessId`'s grammar (`[a-z0-9][a-z0-9._-]{0,63}`) and is
  a distinct type. `BackendVersion` is bounded to 1..=64 printable non-space
  ASCII characters, not semver-parsed — vendors ship `0.3.209`, `1.18.16`, and
  `npm:@scope/pkg@1.2.3`, and rejecting any of those would be Bridge inventing a
  versioning policy it does not own. `InstallationId` is exactly the 24
  lowercase hex characters `managed_payload::installation_id` derives.
- `BackendResolver` holds **many candidates per agent**, ordered by the #166
  backend policy (SDK, structured server, ACP, structured CLI). Registering a
  second candidate for one agent succeeds; registering the same `BackendId`
  twice is rejected.
- `BackendResolver::built_in()` binds exactly the three integrations #161
  proved: `claude` → `claude.agent-sdk`, `codex` → `codex.app-server`,
  `opencode` → `opencode.server`. Each candidate carries the adapter registry
  key it dispatches to, so the resolver selects and the existing
  `AdapterRegistry` still executes.
- A session records its binding — agent, backend, version when the backing
  reports one, installation id when Bridge owns the payload — on the start that
  creates it. `sessions.backend_id`, `sessions.backend_version`, and
  `sessions.backend_installation_id` are added by migration 22 and are nullable.
- **Resume goes through the persisted binding.** Given a stored binding, resume
  resolves that exact `BackendId` rather than re-picking:
  - Backend present, same version → resume normally.
  - Backend present, different version → resume, update the stored version, and
    record a durable fact naming what moved: `backend.version_changed` when the
    version did, `backend.installation_changed` when the installed copy did, and
    both when both. The event kinds are dotted, matching what `events.kind`
    actually contains — an underscore spelling here would send #166's UI work
    looking for a row that does not exist.
  - Backend absent from the resolver → `BackendUnavailable`, naming agent,
    backend, and version. The session's history stays listable and replayable.
  - A *newer preferred* backend exists for the agent → the session still resumes
    through the one it recorded. The divergence is reported, never acted on.
    See the amendment below.
- **A bound session sticks to its backend; it is never blocked by a newer one.**
  Amended after the launch paths were wired — the original rule refused any
  session whose backend differed from a fresh resolution, which reads fine until
  #166 registers a second candidate for `claude` and *every existing Claude
  session* refuses to resume until individually authorized. That contradicts
  #161's delivered promise that existing users do not lose working agents, and
  it makes adding a backend a breaking change. Sticking is also the more literal
  reading of "resume only through the persisted binding": a session that
  continues on the backend it recorded has not chosen a different one.
  `preferred_elsewhere` reports the divergence so a UI can offer the move, and
  the move happens only through an authorization.
- `api::authorize_backend_change` records an explicit authorization for one
  session and one exact transition. A stored authorization that does not match
  the transition being attempted does not permit it; consuming it rebinds the
  session and leaves a durable record of who authorized what.
- A row written before migration 22 has no binding. It is read tolerantly: it
  resumes exactly as it does today and its binding is written on the next
  successful start. No legacy or development row is invalidated, and no
  unbound row is treated as a backend change.

## Unit Tests

- `backend_id_and_agent_id_are_distinct_types` — a `BackendId` cannot be used
  where an `AgentId` is expected, asserted by a compile-fail doc test; both
  accept the same grammar and reject the same values.
- `backend_version_accepts_real_vendor_spellings_and_bounds_the_rest` —
  `0.3.209`, `1.18.16`, `npm:@anthropic-ai/claude-agent-sdk@0.3.209` parse;
  empty, 65-character, whitespace, and control-character values do not.
- `installation_id_matches_what_the_payload_engine_derives` — parsing the value
  `managed_payload::installation_id` produces succeeds, and a value of the wrong
  length, case, or alphabet is rejected.
- `a_resolver_holds_two_backends_for_one_agent_without_collision` — two
  candidates register under one `AgentId`, both are listed in policy order, and
  a duplicate `BackendId` is refused.
- `built_in_agents_resolve_to_the_integrations_161_proved` — the three built-in
  bindings are exactly as above, and each names a candidate whose adapter key is
  registered in `AdapterRegistry::built_in()`.
- `the_backend_table_and_the_built_in_contracts_cannot_drift` — every agent in
  `builtin_compatibility::built_in_agent_contracts()` has exactly one built-in
  backend candidate and vice versa, and each candidate's transport agrees with
  that contract's `transport` field.
- `a_binding_round_trips_through_storage` — write and read back a binding with
  and without version and installation id.
- `an_unbound_legacy_session_reads_as_unbound_not_as_changed` — a row with all
  three columns null yields no binding, and the continuation decision for it is
  "proceed and bind", never `BackendChanged`.
- `resume_through_a_missing_backend_fails_legibly` — the error names agent,
  backend, and version, and is distinguishable from every other resolution
  error without matching message text.
- `a_version_change_resumes_and_records_the_change` — the decision is to
  proceed, the stored version is updated, and the recorded fact carries both the
  old and the new version.
- `losing_the_managed_payload_is_recorded_as_a_move` — clearing the installation
  records `backend.installation_changed` and **only** that: a version line for a
  version that did not move would name nothing that happened.
- `a_move_of_both_version_and_installation_records_both` — when both dimensions
  move, both facts are recorded, in order.
- `a_newer_preferred_backend_does_not_move_or_block_a_bound_session` — with a
  stronger candidate registered, the session resumes through the backend it
  recorded, and `preferred_elsewhere` names the stronger one so a caller can
  offer the move.
- `a_backend_change_happens_only_through_an_authorization` — without one, the
  session stays on its recorded backend; with one for that exact transition, it
  rebinds, records the change, and dispatches to the new backend's adapter.
- `an_authorization_does_not_generalize` — an authorization recorded for one
  transition does not permit a different target backend, a different session, or
  a second unrelated change.
- `no_identity_type_carries_a_credential` — no field in the binding, the
  candidate, or the authorization holds a token, key, path to a credential
  store, or vendor configuration value.

## Integration / Functional Tests

- Migration 22 applies to the current-schema fixture, is idempotent, and leaves
  existing session rows readable with null bindings — asserted through the
  existing migration fixture test rather than a new harness.
- A session started through the normal path records a binding; the same session
  resumed reaches the adapter named by that binding, proven with a recording
  fake registered as a second candidate.
- A session whose backend is removed from the resolver still lists, still
  replays its transcript, and fails only on resume.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` passes,
  including the method-registry and command-signature gates.
- `scripts/check-builtin-adapters.sh` stays green and
  `testing/fixtures/builtin-compatibility-report-v1.json` is unchanged.
- Generated schema and TypeScript artifacts are regenerated and the tree is
  clean afterwards; because no method or DTO changes, the artifacts are expected
  to be byte-identical, which is itself the assertion.
- `bun run build` and `bun run test` pass.

## Smoke Tests

- Start a Codex session, confirm `sessions.backend_id` reads `codex.app-server`
  with a non-null version, stop it, and resume it — the same backend serves it
  and no authorization is demanded.
- Point the resolver at a build with the `codex.app-server` candidate removed
  and confirm the session lists and replays but refuses to resume, naming the
  missing backend.

## E2E Tests

N/A for desktop E2E: this PR adds no user-facing control. The desktop surface
for authorizing a backend change lands with #166, and the flow above is the
core-level equivalent this issue's acceptance asks for.

## Manual / cURL Tests

- N/A for cURL: the transport is a Unix-domain socket and this change adds no
  method to it.
- Manually inspect a persisted binding and confirm it contains no credential,
  token, home path, or vendor configuration value.
