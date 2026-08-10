# codex/issue-172-managed-recipes — Test Contract

## Scope And Stated Assumptions

- Bridge is not a distributor. Payload bytes always come from the vendor's own
  official source; Bridge ships only the pinned version, the expected integrity
  where the publisher provides one, and the resolution logic.
- **Amended during implementation** (see the note at the end of this section):
  all three runtimes install as npm dependency closures. npm is the official
  distribution channel for `@anthropic-ai/claude-agent-sdk`, `@openai/codex`, and
  `opencode-ai`, each publishes the same version there as on its GitHub releases
  page, and each ships its platform binary inside a platform-specific package.
  Every closure is pinned by a committed lockfile, so npm verifies every tarball
  against a recorded SRI integrity hash.
- Closures are installed with `npm ci --ignore-scripts`. The binary ships in the
  platform package, so no vendor postinstall script needs to run, and not running
  them keeps arbitrary vendor code out of the install path.
- npm's `node_modules/.bin` shims are symlinks, which the #174 engine rejects
  outright. They are pruned before the payload is digested: Bridge launches the
  platform binary or loads the module directly and never uses npm's shims.
- An `npm` install tree is not byte-reproducible across machines — all three
  closures pull platform-specific binaries — so there is no honest constant to pin
  the tree against. The supply-chain guarantee is npm's per-tarball integrity from
  the lockfile; the #174 tree digest is computed from the installed result, where
  it still does the job it was built for: proving ownership and catching later
  drift.
- The release-artifact source kind remains supported and tested — a publisher-pinned
  SHA-256 over a single published file — because it is what a future non-npm
  runtime will need. It is simply not what these three use.

- No archive format is accepted implicitly. Entries are validated before being
  written, never after: absolute paths, `..` components, symlinks, hardlinks,
  device nodes, and oversized entries are rejected while extracting, so a hostile
  or malformed archive cannot write outside the staging directory.
- Fetching is injected behind a trait. Every test in this PR runs offline against
  fixtures; no test reaches the network, npm, or a vendor endpoint.
- The real fetcher refuses a plaintext hop (`https_only`, which covers redirects),
  bounds the body while streaming — before any digest can be known — and bounds
  connect and total time. Redirects are allowed but limited, because vendor release
  URLs commonly redirect to a CDN and the bytes are verified against a pinned
  digest before anything uses them.
- `npm ci` runs with the ambient Node-preload variables cleared. `--ignore-scripts`
  stops package lifecycle scripts but not `NODE_OPTIONS`-class injection into the
  install itself.
- Managed payloads live under the leased data directory, registered by
  `BridgeCore::boot` rather than by each host, so a future host cannot forget to do
  it. Until a root is registered the managed tier is inert and the agents behave
  exactly as they did before managed payloads existed.
- Out of scope: RPC (#169), UI (#170), any auth system, any credential storage,
  and any change to how the three integrations talk to their agents.

### Why this changed

The contract originally had Codex and OpenCode installing from GitHub release
archives with publisher-pinned SHA-256 digests. Checking the actual releases
during implementation showed that would be worse on three counts: OpenCode
publishes its CLI only as `.zip` (the `.tar.gz` asset is the desktop app), which
would mean adding a zip extractor; OpenCode publishes no checksums file, so its
digest would have to be one Bridge computed itself, which is not a
publisher-pinned guarantee at all; and both are on npm at the identical version
with SRI integrity already published. Using npm for all three is uniform, keeps
the supply-chain guarantee genuinely vendor-supplied, and drops a dependency.

## Functional Behavior

- A `RuntimeSource` describes where a payload comes from: an npm package at an
  exact version, or an official release artifact (archive or raw binary) at a URL
  with an expected SHA-256. Nothing else is installable.
- Resolution order for every agent is: explicit user-configured executable, then
  the Bridge-managed receipt-bound payload, then a copy bundled with the app,
  then the system PATH. The first hit wins.
- A runtime found on PATH is never claimed as Bridge-managed. It resolves as
  external, stays usable, and is reported through #167's `external` state.
- A user-configured custom executable outranks a Bridge-managed payload, and
  installing or removing a managed payload never reads, writes, moves, or deletes
  it.
- Removing a managed payload cannot alter an external copy. After uninstall, the
  external copy is byte-identical and resolution falls back to it.
- The three integrations are unchanged in how they talk to their agents. Claude
  still runs its existing Node sidecar over the same protocol, Codex still runs
  `codex app-server --listen stdio://`, OpenCode still runs `opencode serve` with
  the same version checks, model discovery, and normalization.
- The Claude sidecar resolves the SDK through an explicit entry when Bridge
  supplies one and through its bundled dependency otherwise, so a managed install
  changes which copy of the SDK is loaded and nothing else. ESM ignores
  `NODE_PATH`, so the managed copy is selected by an explicit module entry rather
  than by environment path injection.
- Vendor authentication is untouched. A vendor reporting a missing login or API
  key surfaces as #167's vendor-owned readiness outcome; Bridge stores, proxies,
  migrates, and deletes nothing.
- A fetch or verification failure leaves no managed payload and no active
  receipt, and reports the failing stage — fetch, integrity, extraction, or
  promotion — rather than a generic error.

## Unit Tests

- `runtime_sources_reject_unpinned_and_unsupported_descriptors` — a floating npm
  range, an empty version, a non-HTTPS URL, and an unknown archive extension are
  all refused before any fetch.
- `release_artifacts_must_match_their_pinned_digest` — a fetched artifact whose
  bytes do not match the recipe's pinned SHA-256 fails, and no payload or receipt
  survives.
- `archive_extraction_rejects_escaping_and_unsupported_entries` — absolute paths,
  `..` traversal, symlinks, hardlinks, device entries, and an entry exceeding the
  size ceiling are each rejected, and nothing is written outside the staging
  directory for any of them.
- `extraction_is_bounded_by_total_size_and_entry_count` — one three-entry archive
  refused three ways through injectable `ExtractLimits`, and accepted under the
  production defaults so the refusals are provably the ceilings. The ceilings are
  injectable precisely because production values are unreachable by any fixture,
  which is how they went untested.
- `pax_size_overrides_cannot_bypass_the_entry_ceiling` — a PAX `size=` record past
  the ceiling with an understating ustar header is refused before anything is
  written; ceilings charge the effective entry size, and the copy is capped
  independently of any header claim.
- `a_raw_binary_release_installs_through_the_payload_engine` — the publisher digest
  verifies the download and the engine's shape-aware digest is what the recipe
  carries, so a raw-binary release actually installs.
- `npm_wildcards_are_not_exact_versions` — `x`/`X` wildcards, ranges, `latest`, and
  partial versions are all refused; exact and prerelease versions are accepted.
- `platform_naming_follows_each_vendors_own_spelling` — Anthropic and OpenAI
  publish `win32-*`, OpenCode publishes `windows-*`, Windows leaves carry `.exe`,
  and each recipe's platform package is one its own lockfile pins.
- `registering_a_root_makes_the_managed_tier_live` — the managed tier is inert
  until a host registers a root, live once it has, re-registerable so a second
  `boot` cannot resolve out of the first core's directory, and inert again when the
  payload drifts.
- `a_failed_prepare_leaves_no_staging_residue` — a failed attempt leaves nothing at
  its staging path for a retry to adopt.
- `npm_closure_installs_are_pinned_by_lockfile_not_by_tree_digest` — an npm-shaped
  source is installed with an exact version and a lockfile, and its recorded
  integrity is the digest of the resulting tree rather than a pinned constant.
- `npm_bin_symlinks_are_pruned_before_digesting` — a staged closure containing
  `node_modules/.bin` symlinks is pruned so the payload engine accepts it, and
  nothing outside `.bin` is removed.
- `each_recipe_targets_a_platform_binary_that_exists_in_its_closure` — the Claude,
  Codex, and OpenCode entrypoints name the platform package's executable, and the
  platform component is resolved for the host rather than hardcoded.
- `resolution_prefers_explicit_then_managed_then_bundled_then_path` — all four
  tiers present resolves to the explicit one; removing tiers walks the order
  down; nothing present is an error naming the agent.
- `path_runtimes_are_never_claimed_as_managed` — with only a PATH runtime, the
  agent is `external`, no receipt exists anywhere under the managed root, and the
  binary is untouched.
- `a_custom_executable_outranks_a_managed_payload_and_is_never_touched` — with
  both configured, the custom one is chosen; installing and uninstalling the
  managed payload leaves it byte-identical.
- `uninstall_falls_back_to_the_external_copy_without_altering_it` — after removal
  resolution returns the external path and its bytes are unchanged.
- `each_agent_recipe_pins_an_exact_version_and_entrypoint` — the Claude, Codex,
  and OpenCode recipes each validate through #174's `PayloadRecipe::validate`,
  name an exact version, and declare a relative entrypoint.
- `a_failed_fetch_leaves_no_payload_and_names_the_stage` — fetch, integrity, and
  extraction failures each report their own stage and leave no active receipt.
- `managed_installs_do_not_change_the_162_contract` — capabilities, sandbox modes,
  and model lists are identical with and without a managed payload, which is what
  the #162 gate asserts. The reported *version* is deliberately not invariant: it
  describes whichever copy will actually launch, so a managed payload is never
  described by whatever happens to be on PATH. The gate does not snapshot version,
  so this cannot move it.

## Integration / Functional Tests

- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core managed_runtime`
  passes offline against fixture archives and a fake fetcher.
- The three recipes drive a real `ManagedPayloadStore` and #167 coordinator
  end-to-end from a fixture artifact: `not_installed → installing → installed →
  ready`, then uninstall back to `external` or `not_installed`.
- `scripts/check-builtin-adapters.sh` is green before install, after install, and
  after uninstall for all three agents — the #162 gate this issue must not move.
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core` passes.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` passes.
- `bun run build` passes.
- `bun run test` passes, including the Claude sidecar suite with its SDK entry
  override exercised.

## Smoke Tests

- With no managed payload and a runtime on PATH, confirm the agent reports
  external and starts through its existing integration exactly as before.
- Install a managed payload from a fixture artifact, confirm the agent resolves to
  the managed path, start it, then uninstall and confirm resolution returns to the
  external copy with the external bytes unchanged.
- Point a recipe at an artifact whose digest does not match and confirm the
  managed root is left with no installation and no active receipt.

## E2E Tests

N/A for desktop E2E in this PR: install and remove are not user-reachable until
the RPC surface in #169 and the Plug / Play / Remove UI in #170.

## Manual / cURL Tests

- N/A for cURL: this PR adds no RPC or HTTP surface. The only outbound HTTP is the
  vendor fetch, which is injected and never exercised in tests.
- Manually confirm the pinned versions in the recipes match the versions the
  vendors publish, and that each expected digest was taken from the vendor's own
  published artifact rather than computed locally.
- Manually confirm no recipe, receipt, or log line contains a credential, token,
  home path, or vendor configuration value.
