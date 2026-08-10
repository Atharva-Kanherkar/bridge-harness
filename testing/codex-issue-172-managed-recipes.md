# codex/issue-172-managed-recipes — Test Contract

## Scope And Stated Assumptions

- Bridge is not a distributor. Payload bytes always come from the vendor's own
  official source; Bridge ships only the pinned version, the expected integrity
  where the publisher provides one, and the resolution logic.
- Two integrity models, because the three payloads are not the same shape, and
  this is a deliberate decision rather than an oversight:
  - **Codex and OpenCode** are single official release artifacts, so the recipe
    carries a publisher-pinned SHA-256 that must match before anything is
    promoted. A mismatch is a hard failure.
  - **Claude** is the `@anthropic-ai/claude-agent-sdk` dependency *closure*, and
    an `npm` install tree is not byte-reproducible across machines or npm
    versions. Its supply-chain guarantee therefore comes from npm's own
    per-tarball integrity, pinned by a committed lockfile and installed with
    `npm ci`; the #174 tree digest is computed from the installed tree and
    recorded in the receipt, where it still does the job it was built for —
    detecting later drift and proving ownership.
- No archive format is accepted implicitly. Entries are validated before being
  written, never after: absolute paths, `..` components, symlinks, hardlinks,
  device nodes, and oversized entries are rejected while extracting, so a hostile
  or malformed archive cannot write outside the staging directory.
- Fetching is injected behind a trait. Every test in this PR runs offline against
  fixtures; no test reaches the network, npm, or a vendor endpoint.
- Out of scope: RPC (#169), UI (#170), any auth system, any credential storage,
  and any change to how the three integrations talk to their agents.

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
- `extraction_is_bounded_by_total_size_and_entry_count` — an archive over either
  ceiling is refused rather than filling the disk.
- `npm_closure_installs_are_pinned_by_lockfile_not_by_tree_digest` — an npm-shaped
  source is installed with an exact version and a lockfile, and its recorded
  integrity is the digest of the resulting tree rather than a pinned constant.
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
- `managed_installs_do_not_change_adapter_descriptors` — descriptor capabilities,
  sandbox modes, and model lists are identical with and without a managed
  payload, so the #162 contract cannot drift.

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
