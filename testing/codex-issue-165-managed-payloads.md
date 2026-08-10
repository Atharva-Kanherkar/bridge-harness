# codex/issue-165-managed-payloads — Test Contract

## Functional Behavior

- This PR adds only the fixture-backed managed payload engine required by #165. It does not add real Claude, Codex, or OpenCode recipes; change adapter resolution; add RPC/UI; download from the network; or manage vendor authentication.
- The managed root is supplied by the caller. Bridge stores payloads only below that root using stable, validated agent/version/platform components; absolute paths, traversal, separators, empty identifiers, and symlinked artifact entries are rejected.
- A recipe names an agent, exact version, platform, source label, expected SHA-256 tree digest, payload source, and relative executable entrypoint. File and directory payload shapes are supported for offline fixtures.
- Install copies into a unique sibling staging directory, rejects unsupported filesystem entries, verifies the staged digest and entrypoint, writes an embedded receipt, syncs files required for durability, then atomically renames the complete installation into its immutable final directory.
- The embedded receipt and atomically written active receipt contain schema version, agent, exact version, platform, source, integrity, installation id, relative owned paths, relative entrypoint, and installation timestamp. Receipt paths are relative to the managed root and never authorize arbitrary absolute deletion.
- A crash before promotion leaves no active installation. A crash after promotion but before the active receipt leaves an embedded receipt that a retry of the identical recipe can verify and activate without recopying or guessing ownership.
- Repeating the same install converges to `AlreadyInstalled`. Concurrent installs for the same managed root and agent are serialized and produce one valid active installation. Operations for different agents do not share ownership state.
- Status distinguishes `NotInstalled`, `Installed`, and `Repairable`. Missing payloads, corrupt receipts, receipt mismatches, missing entrypoints, and integrity drift are visible repair reasons rather than being silently treated as installed.
- Repair is allowed only when a valid active or embedded receipt proves Bridge ownership of the deterministic installation path. Repair never adopts an arbitrary pre-existing directory or an external runtime.
- Uninstall reads and validates Bridge's receipt chain, verifies every owned path is a safe relative descendant of the managed root and matches the installation being removed, and removes only that receipt-owned immutable installation plus the active receipt. Missing installations converge to `AlreadyAbsent`.
- A corrupt/mismatched receipt, unproven directory, symlinked managed ancestor, or owned path outside the exact installation causes uninstall to fail closed without deleting payload or external files.
- External/user-managed runtime detection is read-only. It reports matching caller-supplied executable candidates separately and never writes a receipt, copies, changes, or removes those paths.
- Uninstall never touches Bridge session/history storage. Existing #162 compatibility tests remain green before and after this module lands.

## Unit Tests

- `recipe_rejects_unsafe_components_entrypoints_and_symlink_sources` — unsafe identifiers, escaping entrypoints, and source symlinks fail before staging becomes active.
- `file_and_directory_payloads_install_with_versioned_receipts` — both fixture shapes install below the managed root with exact receipt fields, executable entrypoints, and matching staged integrity.
- `tampered_artifact_never_becomes_active` — a digest mismatch returns an integrity error and leaves no active receipt or installation.
- `repeated_and_concurrent_installs_converge_to_one_owned_installation` — identical requests serialize per agent, create one immutable installation, and later requests return `AlreadyInstalled`.
- `retry_recovers_a_promoted_installation_from_its_embedded_receipt` — a simulated failure after atomic promotion but before active receipt publication is recovered by retry without losing ownership proof.
- `status_reports_corrupt_missing_and_drifted_installations_as_repairable` — receipt parse failure, absent payload, missing entrypoint, and modified payload produce bounded typed repair reasons.
- `repair_requires_receipt_proof_and_restores_a_drifted_owned_payload` — verified ownership permits repair; an unreceipted directory is never adopted or removed.
- `uninstall_removes_only_the_exact_receipt_owned_installation` — managed payload and active receipt disappear while sibling versions, external files, and unrelated managed-root content remain.
- `uninstall_fails_closed_for_corrupt_or_forged_receipts` — absolute/traversing/wrong-installation owned paths and receipt disagreement cannot authorize deletion.
- `repeated_uninstall_converges_and_external_detection_never_claims_ownership` — second uninstall returns `AlreadyAbsent`; external candidates remain unchanged and receipt-free.
- `different_agents_have_independent_serialization_and_receipts` — installing two agents produces separate active receipts and no cross-agent mutation.

## Integration / Functional Tests

- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core managed_payload` passes with offline temporary fixtures only.
- `scripts/check-builtin-adapters.sh` remains green, proving the Claude, Codex, and OpenCode compatibility report and representative normalization streams did not drift.
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core` passes.
- `bun run build` passes.
- `bun run test` passes, including the Claude sidecar, frontend, and complete Rust workspace suites.

## Smoke Tests

- Install a fixture directory, inspect status, resolve the active entrypoint, uninstall it, verify unrelated/external files remain, then reinstall the same recipe successfully.
- Inspect the managed root after a failed integrity check and confirm there is no active receipt and no promoted installation for that recipe.
- N/A for authenticated vendor smoke in this PR: real agent recipes and vendor prerequisites land in #172. Existing opt-in #162 smoke tests remain unchanged.

## E2E Tests

- N/A for desktop E2E in this engine-only PR. RPC and desktop Plug / Play / Remove flows land in #169 and #170.

## Manual / cURL Tests

- N/A for cURL: this PR adds no RPC or HTTP surface.
- Manually inspect a generated fixture receipt and confirm all owned paths and the entrypoint are relative, every identity/version/platform field matches the recipe, and no credential, environment value, home path, or external runtime path is stored.
- Manually compare filesystem trees before and after uninstall to confirm only the exact receipt-owned installation and active receipt were removed.
