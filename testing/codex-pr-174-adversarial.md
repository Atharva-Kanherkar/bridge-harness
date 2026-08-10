# codex/pr-174-adversarial — Test Contract

Adversarial review of the managed payload engine added by #165. Ten confirmed
defects, fixed here. Nothing outside `managed_payload.rs` changes behaviour, and
the eleven contract tests from `codex-issue-165-managed-payloads.md` stay green
unmodified.

## Functional Behavior

- Provenance is not identity. `source` records where the installed bytes came
  from; `integrity_sha256` pins which bytes they are, and `installation_id` is
  derived without `source`. Reinstalling a byte-identical artifact under a new
  provenance label converges to `AlreadyInstalled`/`AlreadyHealthy` instead of
  mapping onto the same directory and then failing its own ownership check.
- A payload's digest describes the logical tree that lands under `payload/`:
  entries sorted by canonical relative path, folded in as `dir\0<path>\0` or
  `file\0<path>\0<len><bytes>`. Paths are `/`-joined and must be valid UTF-8;
  directories are covered, so an added or removed empty directory is drift; file
  bytes stream through the hasher so memory stays flat on payloads of any size.
- Both payload shapes verify the whole installed tree. Hashing only the
  entrypoint left everything else under `payload/` outside integrity.
- Install verifies the representation it stored rather than the source it was
  promised, hashing the payload once instead of twice, and creates the agent's
  `installations` directory only once the staged tree has proven itself.
- Permission bits stay outside the digest — they do not survive every transport
  and would make a digest platform-specific — so install asserts the entrypoint
  is executable and status reports `EntrypointNotExecutable` when that stops
  being true.
- Superseded installations are reclaimed: on install for the version it
  supersedes, on uninstall for every remaining installation of that agent whose
  own receipt proves Bridge wrote it. Directories with no such proof are never
  pruned.
- Staging lives at `.staging/<agent>/<id>`. Whatever a killed process left in
  this agent's staging directory is discarded before the next install; another
  agent's staging directory is never a candidate, including an agent whose id
  shares a prefix.
- The per-agent lock guards no data, so poisoning it is tolerated. A panic under
  the lock does not brick install, repair, or uninstall for the rest of the
  process. `status` takes the same lock rather than reading torn state.
- Repair may replace the deterministic installation path when a receipt proves
  Bridge created it, but only while the directory still has the shape Bridge
  writes — a `payload` directory, and nothing at the top level except that and
  `receipt.json`. A directory holding foreign content fails closed.
- Receipt reads are bounded, and a receipt that does not name exactly one owned
  path is a typed repair reason rather than an index panic.
- The entrypoint's own ancestry is checked for symlinks, not just its leaf.

## Unit Tests

- `relabelled_provenance_converges_instead_of_wedging_the_agent` — identical
  installation id across labels; install and repair converge; the receipt keeps
  the provenance of the bytes as first installed.
- `integrity_covers_the_whole_payload_tree_for_both_shapes` — source digest and
  installed-tree digest agree; a file planted beside a file-shaped entrypoint is
  drift; a removed empty directory is drift.
- `digest_format_is_pinned_and_platform_independent` — the digest of a fixed
  tree is pinned to a constant, and non-UTF-8 relative paths are rejected rather
  than folded to `U+FFFD` where distinct trees could collide.
- `entrypoint_executability_is_enforced_and_reported` — a source that lost its
  permission bits still installs runnable; a stripped bit reports
  `EntrypointNotExecutable`; repair restores it.
- `superseded_installations_are_reclaimed_but_unproven_ones_are_not` — upgrade
  reclaims the old version, uninstall reclaims a receipt-owned orphan, and a
  hand-made sibling survives both.
- `staging_orphans_are_reaped_without_crossing_agents` — this agent's staging
  orphan is discarded; a prefix-sharing agent's staging directory is untouched.
- `lifecycle_survives_a_poisoned_agent_lock` — status, install, repair, and
  uninstall all still work after a panic under the lock.
- `repair_refuses_to_replace_a_directory_holding_foreign_content` — a corrupt
  embedded receipt stays recoverable, but not at the cost of deleting a user
  file that shares the directory.
- `install_verifies_the_stored_representation` — a source tampered after recipe
  resolution fails the staged check, leaves no `installations` directory, no
  active receipt, and no staging residue.
- `receipts_are_bounded_and_owned_paths_are_checked` — an oversized active
  receipt is `CorruptActiveReceipt`; zero or duplicate owned paths are
  `ActiveReceiptMismatch`; neither deletes the installation.

## Integration / Functional Tests

- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core managed_payload`
  passes with 23 tests — the 13 from #165 unmodified, plus the 10 above.
- `cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core` passes.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` passes.
- `scripts/check-builtin-adapters.sh` remains green, proving the Claude, Codex,
  and OpenCode compatibility report and representative normalization streams did
  not drift.
- `bun run build` and `bun run test` pass.

## Smoke Tests

- Install a fixture payload, strip the entrypoint's executable bit, confirm
  status reports `EntrypointNotExecutable`, repair, and confirm the entrypoint
  runs again.
- Install a version, install a second version of the same agent, and confirm the
  superseded installation is gone while an unreceipted sibling directory remains.
- Interrupt an install, then reinstall, and confirm no `.staging` residue for
  that agent survives.

## E2E Tests

N/A for desktop E2E in this engine-only change. RPC and desktop Plug / Play /
Remove flows land in #169 and #170.

## Manual / cURL Tests

- N/A for cURL: this change adds no RPC or HTTP surface.
- Measured before/after on a 320 MB single-file payload: resident memory during
  install went from the full payload size to flat, and install hashes the payload
  once rather than twice.

## Deliberately Out Of Scope

- Caching status digests. Status still rehashes the payload on every call; a
  correctness-preserving cache needs an invalidation design and belongs with
  #169/#170, where call frequency is known.
- Migrating this module's symlink checks onto `cap-std` (already used by
  `workspace_files`). The checks stay check-then-use, and `sync_directory` stays
  a no-op off Unix. Both are now recorded in the module docs rather than implied
  to be stronger than they are.
