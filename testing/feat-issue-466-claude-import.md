# feat/issue-466-claude-import — Test Contract

## Functional Behavior

- A provider-neutral import model represents sources, discovery results, candidates,
  plans, validation, commits, per-candidate outcomes, provenance, diagnostics,
  redaction summaries, confidence, and stability without Claude-specific fields in
  the shared contract.
- `ExternalHarnessImporter` implementations discover only explicit, canonicalized,
  user-approved roots or exports. Discovery is read-only, returns metadata/counts,
  does not persist candidate content, and does not invoke a source harness, API,
  network connection, hook, command, skill, agent, plugin, or MCP server.
- The Claude Code importer recognizes documented project/user instruction locations
  (`CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md`, and Markdown below approved
  `.claude/rules` roots) and normalizes them into disabled, review-only instruction
  or rule setup candidates.
- Claude auto-memory parsing is available only through an explicit allowlisted
  storage/schema gate. Each imported note retains Claude auto-memory provenance,
  requires a selected Bridge scope, and exposes memory conflicts for an explicit
  `keep existing`, `import alongside`, or `skip` decision.
- Claude transcript parsing accepts only allowlisted, fixture-pinned JSONL schema
  variants. Supported user/assistant text and safe tool metadata preserve source
  order, timestamps, source IDs, and project hints. Unknown versions and malformed,
  duplicate-ID, or unsupported unsafe variants fail closed before any write.
- Claude settings, commands, skills, subagents, hooks, and MCP definitions normalize
  only into disabled setup candidates. Import never applies environment values,
  executes content, authenticates, connects, installs, or changes active prompts.
- Structured credential/auth fields are excluded before payload inspection. A
  conservative scanner then redacts secret-shaped content. Candidates that cannot
  be represented safely are excluded with category/path-only diagnostics; raw
  rejected content and secret values never enter persisted payloads or diagnostics.
- Deterministic Bridge IDs use a versioned importer namespace, provider, canonical
  source reference, stable source-native ID when present, and canonical content
  hash. Claude source IDs are never used directly as Bridge primary keys.
- The import ledger makes an unchanged re-import a no-op. Changed source content is
  a distinct reviewable revision and requires `skip` or `import as new historical
  revision`; historical records are never overwritten automatically.
- All selected candidates are validated before the write transaction. Imported
  sessions, immutable entries, memories, disabled setup candidates, provenance,
  and ledger rows commit atomically. An injected failure leaves none of those rows.
- Imported conversations are visibly marked `Imported from Claude Code`, retain a
  non-identifying source-path fingerprint, are immutable historical sessions, and
  never claim or attempt provider-native resume. Starting fresh requires a separate
  explicit user action.
- The Import screen states that source reads and Bridge records remain local, local
  databases are not encrypted backups, setup stays disabled, and sources are never
  changed. Preview supports conservative per-item selection and distinguishes
  history, memory, reusable setup, warnings, and not-imported diagnostics. Results
  distinguish imported, duplicate, changed, conflicted, rejected, and unsupported.
- The shared API and storage are reusable by Codex, OpenCode, and Cursor adapters;
  provider adapters own discovery/normalization only, while canonicalization,
  redaction, selection, identities, conflict handling, transactions, ledger,
  diagnostics, and persistence remain in the shared layer.

## Unit Tests

- `external_import::tests::deterministic_identity_is_stable_and_namespaced` — stable
  inputs reproduce an ID; changed content and providers produce different IDs.
- `external_import::tests::secret_filter_excludes_structured_auth_and_redacts_text`
  — auth/env fields and secret corpus values cannot survive in payloads or messages.
- `external_import::tests::unknown_schema_versions_fail_closed` — private/offline
  formats outside an allowlist return actionable export/manual-import diagnostics.
- `claude_import::tests::discovers_documented_markdown_inside_approved_roots` — all
  documented instruction/rule locations normalize with stable classifications.
- `claude_import::tests::rejects_paths_outside_approved_roots` — traversal,
  symlink escape, and unapproved home/project locations are excluded.
- `claude_import::tests::gated_memory_preserves_type_provenance_and_scope` — a pinned
  auto-memory fixture becomes an unselected scoped memory candidate.
- `claude_import::tests::gated_jsonl_preserves_order_roles_timestamps_and_ids` — a
  pinned transcript fixture creates one historical conversation and safe entries.
- `claude_import::tests::rejects_malformed_unknown_and_duplicate_jsonl_records` —
  invalid or unsupported transcript data fails before normalization/persistence.
- `claude_import::tests::setup_candidates_are_inert_and_secret_free` — settings,
  command, skill, agent, hook, and MCP fixtures remain disabled review candidates.
- Frontend tests verify conservative selection, warning/status presentation,
  local-only copy, disabled setup labels, and the persistent Claude provenance badge.

## Integration / Functional Tests

- A temporary approved Claude project/config tree flows through discover → preview →
  validate → normalize → commit and produces historical session entries, scoped
  memory records, disabled setup candidates, provenance, diagnostics, and ledger rows.
- Re-running the unchanged plan creates no records and reports duplicates.
- Modifying source content reports `changed_source`; explicit revision import creates
  new historical records linked by provenance without altering the original.
- Existing scoped memory/setup conflicts require an explicit policy and preserve a
  conflict group; no auto-merge or overwrite occurs.
- Failure injection after each destination write proves the encompassing SQLite
  transaction rolls back sessions, entries, memories, setup candidates, and ledger.
- Generated protocol schemas, Rust command registration, TypeScript types, native
  API delegation, and browser-mode mock remain synchronized for discovery, preview,
  and commit operations.

## Smoke Tests

- `cargo test -p bridge-core external_import`
- `cargo test -p bridge-core claude_import`
- Targeted frontend import/provenance component tests pass.
- `bun run build` succeeds.
- `bun run test` succeeds, including the full Rust and Vitest suites.
- `git diff --check origin/main...HEAD` reports no whitespace errors.

## E2E Tests

- In browser/mock mode, open Import, choose Claude Code, preview representative
  history/memory/setup items, leave conservative defaults unchanged, confirm the
  exact plan, and verify the result categories and immutable imported-session badge.
- N/A for live Claude data: automated tests use sanitized temporary fixtures only;
  they never inspect a developer's actual Claude home or start Claude Code.

## Manual / cURL Tests

- Start `bun run dev`, open the Import screen, and verify the local-only consent,
  explicit root/export choice, preview grouping, disabled setup state, final commit
  summary, and imported-session provenance presentation.
- Inspect the temporary integration-test SQLite database and confirm one atomic set
  of destination/ledger rows after success and zero partial rows after failure.
- Search committed fixtures and source for the secret-corpus canaries and confirm no
  literal credential value appears in snapshots, diagnostics, or generated schemas.
- No cURL test is applicable: the importer is a local Tauri/typed-daemon operation
  and deliberately exposes no network endpoint.
