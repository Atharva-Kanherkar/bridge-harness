# codex/issue-164-166-verified-catalog — Test Contract

Locks #164 (the Bridge Verified compatibility catalog) and #166 (the Bridge-owned
integration framework with pluggable backend drivers), the P1 and P2 of the
marketplace epic #171. They land together because #166's central rule — a
runtime's advertised capabilities are *evidence checked against the verified
profile*, never authority to unlock behaviour — has no meaning until the profile
exists, and a catalog with nothing that consumes it is a schema, not a contract.

Builds directly on #163 (#186): `AgentId`/`BackendId`/`BackendVersion`/
`InstallationId`, the `BackendResolver`, and the session binding.

## Scope And Stated Assumptions

- **The catalog is Bridge's, not a mirror.** `acp_registry` stays exactly what it
  is — a tolerant reader of an upstream index, useful for discovering that an
  agent exists. Nothing in it may produce a Bridge Verified entry, and a test
  asserts no code path converts a `RegistryAgent` into one. Upstream publication
  is an input to a human decision, never a substitute for one.
- **A new dependency: `ed25519-dalek`.** #164 requires an authenticated remote
  snapshot, and the workspace has no signature verification of any kind today —
  `sha2` proves a file is the file you expected, not that Bridge published it.
  Ed25519 is the smallest thing that closes that gap: fixed-size keys, no
  parameter negotiation, no curve selection, and therefore no configuration a
  caller can get wrong. Verification only; Bridge never signs on the desktop.
- **`RuntimeSource::NpmClosure` moves from `&'static str` to `Cow<'static, str>`
  for `manifest` and `lockfile`.** A recipe delivered as catalog data cannot
  produce a `&'static str` without leaking. This is the one change this PR makes
  to #174's engine, it is mechanical and type-checked, and every existing
  built-in recipe keeps passing `&'static str` unchanged via `Cow::Borrowed`.
  The alternative — a parallel recipe type that converts at the boundary — would
  duplicate the validation that makes the engine safe.
- **A snapshot replaces last-known-good only in full.** Validation is
  all-or-nothing across the document: one malformed entry rejects the whole
  snapshot rather than loading the rest. A partially applied catalog is a
  catalog nobody can reason about, and #164 names exactly this case.
- **Profiles are data.** No catalog type has a field that can carry a command,
  an argv, a script, a shell string, an interpreter, or a path to execute.
  Executable behaviour lives only in named integration modules compiled into
  Bridge. A test enumerates the catalog's field names and fails on any that
  could smuggle execution.
- **Vendor authentication stays vendor-owned.** The catalog may say *what* a
  vendor requires ("run `codex login`", "set `OPENAI_API_KEY`"); it may not
  contain a secret, and no type here has a field a credential could live in.
- **The three built-ins are not rewritten.** `claude`, `codex`, and `opencode`
  keep their hand-written adapters and their #161-proven behaviour.
  `builtin_compatibility` gains no field, `builtin-compatibility-report-v1.json`
  stays byte-identical, and `scripts/check-builtin-adapters.sh` stays green. The
  integration framework is demonstrated by *fake* integrations, per #166's own
  acceptance.
- **No new agent, and no RPC method.** The catalog is readable through
  `bridge_core::api`; exposing it on the wire belongs with the UI that consumes
  it. Recorded so its absence reads as a decision.
- Out of scope: #168's verification pipeline and staged promotion. This PR
  *models* verification status and refuses to promote an unverified entry; it
  does not produce evidence. Nothing here decides that an entry is verified.

## As Built: Where The Code Departs From This Contract

Two deviations, both deliberate, recorded here so a reviewer reading the
contract against the diff does not have to guess whether they were decisions.

- **`Catalog::load` is `CatalogStore::load`.** The catalog in force and the
  place it is cached are different concerns: `Catalog` is the verified document,
  `CatalogStore` owns the directory it survives a restart in. Loading is a
  question about the *store* — it is the thing that knows whether a cache exists
  — so it lives there. `Catalog::bundled` and `Catalog::install_snapshot` are
  unchanged and still the only ways a catalog comes into being.
- **"Its backend is not one the build can reach" is not entry validation.** It
  was listed under the conditions that fail an entry, which would mean an older
  Bridge refuses a whole snapshot for naming an agent a newer Bridge supports —
  making every future agent a breaking change for every build that predates it.
  The same document is legitimately valid on the build that ships the
  integration and the one that does not, so the question is asked at
  registration instead: `IntegrationRegistry::offer_catalog` skips the entry,
  reports `no_integration`, and the rest of the catalog serves.

Also worth naming, because both went further than the contract asked:
`offer_catalog` cannot fail at all — a snapshot arrives from the network, and no
entry in one may leave Bridge unable to resolve the agents it already had — and
`AgentIntegration` declares its own resume support, so an agent whose runtime
cannot resume is not asked to merely because its transport could.

## Functional Behavior

### The catalog (#164)

- A `VerifiedEntry` carries: stable `AgentId`; display/vendor/license/source
  metadata; one exact `BackendVersion`; the supported platform matrix; a typed
  install recipe convertible to `managed_runtime::RuntimeSource`; integrity
  evidence; the selected `BackendId` and its `BackendKind`; Bridge-owned
  integration configuration; vendor setup guidance; expected capabilities;
  known-blocked versions; the compatible Bridge version range; and verification
  status with suite version, timestamp, and an evidence reference.
- A `CatalogSnapshot` is a signed document with a schema version and a
  monotonic generation. `Catalog::load` takes the bundled bootstrap first, then
  replaces it with a cached snapshot only if that snapshot still validates.
- `install_snapshot` verifies an Ed25519 signature over the **exact bytes**
  received, against a compiled-in public key, before parsing anything. It then
  validates the whole document and only then replaces the cache atomically,
  recording provenance: the byte digest, the signing key id, and when it was
  fetched.
- A snapshot is refused, leaving last-known-good untouched, when: the signature
  does not verify; the key is unknown; the schema version is not supported; the
  generation is not greater than the installed one (no rollback, no replay);
  the snapshot declares a Bridge version range this build is outside; or **any**
  entry fails validation.
- An entry fails validation when its recipe is unpinned or not HTTPS, its
  integrity evidence is missing or malformed, its backend is not one the build
  can reach, its platform matrix is empty, its version appears in its own
  blocked list, or its verification status is not `Verified`.
- The cached snapshot is stored as the exact bytes received plus its detached
  signature, so a reload re-verifies rather than trusting the parse that
  admitted it.

### The integration framework (#166)

- `AgentIntegration` is the boundary a supported agent implements: descriptor
  and model mapping, launch configuration, event normalization, its permission
  model, its resume semantics, and a compatibility check against a
  `VerifiedEntry`.
- Four reusable driver primitives cover the backend policy: a long-lived SDK
  sidecar, a structured JSON-RPC/NDJSON subprocess, an ACP server, and a bounded
  structured CLI. Each owns transport, framing, and bounded failure reporting;
  none owns agent-specific behaviour.
- An integration may declare narrow typed extension handlers for vendor-specific
  official methods. An unknown method is reported, never silently dropped and
  never dispatched.
- **Capabilities are evidence, not authority.** A runtime that advertises a
  capability its verified profile does not list has that capability *reported as
  drift* and refused. A runtime that advertises fewer is degraded, not blocked.
  Neither case lets the runtime widen what Bridge will do with it.
- The control plane is one path: three fake integrations — SDK-sidecar,
  structured-server, and ACP shaped — run through the same launch, turn,
  permission, interrupt, resume, and shutdown surface.
- Adding a future agent requires a catalog profile, a named integration module,
  and tests. It requires no change to the orchestrator, the router, the session
  store, or the delegation pipeline — asserted structurally, not asserted by
  assertion.

## Unit Tests

### Catalog

- `a_snapshot_must_be_signed_by_a_known_key` — a valid signature from the
  compiled-in key is accepted; a wrong signature, a signature over different
  bytes, and a signature from an unknown key are each refused with distinct
  codes, and last-known-good is untouched in all three.
- `signature_is_verified_over_exact_bytes_before_parsing` — a document that is
  not even valid JSON is refused for its *signature* first, proving nothing
  parses a snapshot Bridge has not authenticated.
- `one_bad_entry_rejects_the_whole_snapshot` — a snapshot of five entries, one
  invalid, installs none of them and leaves the previous catalog serving.
- `a_snapshot_cannot_roll_the_catalog_back` — a generation equal to or lower
  than the installed one is refused, so a replayed older snapshot cannot
  reinstate a withdrawn entry.
- `a_snapshot_outside_this_builds_version_range_is_refused` — and the refusal
  names the range, because the fix is upgrading Bridge.
- `an_unverified_entry_never_loads` — an entry whose verification status is not
  `Verified` fails validation, so #168 gates promotion by writing status rather
  than by being trusted to filter.
- `upstream_registry_entries_cannot_become_verified_entries` — no public
  constructor, `From`, or conversion turns a `RegistryAgent` into a
  `VerifiedEntry`, asserted structurally over the module's API.
- `no_catalog_type_can_carry_an_executable` — every field of every catalog type
  is enumerated; none is named or typed such that a command, argv, script,
  shell string, interpreter, or executable path could be delivered as data.
- `no_catalog_type_can_carry_a_credential` — the same enumeration for tokens,
  keys, and secrets; vendor setup guidance is prose and a typed requirement
  kind, never a value.
- `a_catalog_recipe_becomes_the_engine_s_runtime_source` — a recipe from catalog
  data converts to `RuntimeSource` and passes `RuntimeSource::validate`, and an
  unpinned or non-HTTPS one is refused by the same validation the built-ins face.
- `built_in_recipes_are_unchanged_by_the_cow_change` — the three built-in
  recipes produce byte-identical `RuntimeSource` values before and after
  `manifest`/`lockfile` become `Cow`, asserted against their pinned versions.
- `the_bundled_bootstrap_catalog_validates` — the compiled-in last-known-good
  document passes exactly the validation a remote snapshot must pass. A
  bootstrap that could not be installed as a snapshot is not a catalog.

### Integration framework

- `three_backend_shapes_run_through_one_control_plane` — fake SDK-sidecar,
  structured-server, and ACP integrations each complete launch → turn →
  interrupt → resume → shutdown through the same surface, with no shape-specific
  branch in the caller.
- `an_integration_reports_permissions_rather_than_deciding_them` — a permission
  request surfaces as a typed request; the integration never self-approves.
- `an_interrupt_is_delivered_or_reported_as_unsupported` — an integration whose
  transport cannot interrupt says so; it does not silently drop the interrupt.
- `resume_semantics_are_declared_and_honoured` — an integration declaring no
  native resume is never asked to resume, matching how `AdapterRegistry` already
  refuses.
- `a_dead_process_reports_why_it_died` — bounded stderr and exit status reach
  the caller, matching the `failure_context` contract the built-ins already meet.
- `normalization_is_total_over_a_fixture_stream` — every frame in a recorded
  fixture normalizes or is explicitly reported as unknown; nothing is dropped.
- `advertised_capabilities_beyond_the_profile_are_refused_as_drift` — a runtime
  claiming a capability its verified entry does not list is refused with a
  distinct code; claiming fewer degrades instead.
- `an_unknown_extension_method_is_reported_not_dispatched`.
- `adding_an_agent_touches_only_a_profile_and_an_integration` — registering a
  fake integration plus a catalog entry makes it launchable through the existing
  control plane with no change to orchestrator, router, session store, or
  delegation modules, asserted structurally.

## Integration / Functional Tests

- A fake agent registered through catalog + integration resolves a backend
  through #163's `BackendResolver`, binds its session, and resumes through that
  binding — the two PRs' seams meeting.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` passes,
  including the protocol registry and command-signature gates.
- `scripts/check-builtin-adapters.sh` stays green and
  `testing/fixtures/builtin-compatibility-report-v1.json` is unchanged.
- Generated schema and TypeScript artifacts are regenerated and the tree is
  clean; no method or wire DTO changes here, so a zero diff is the assertion.
- `bun run build` and `bun run test` pass.
- `cargo clippy --workspace --all-targets` reports nothing new on touched files.

## Smoke Tests

- Install a signed fixture snapshot over the bundled bootstrap, confirm the new
  entry is served and the provenance records its digest and key id; then install
  a tampered copy of the same bytes and confirm the previous catalog still
  serves.
- Drive a fake ACP integration through a full turn and confirm the normalized
  events are indistinguishable in shape from a built-in adapter's.

## E2E Tests

N/A for desktop E2E: no user-facing surface lands here. The catalog's UI and its
RPC method arrive with the marketplace screen; the flows above are the
core-level equivalent this PR's acceptance asks for.

## Manual / cURL Tests

- N/A for cURL: the transport is a Unix-domain socket and this PR adds no method
  to it.
- Manually inspect a cached snapshot and its provenance record and confirm
  neither contains a credential, a token, a home path, or a vendor configuration
  value.
