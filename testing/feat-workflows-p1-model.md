# feat/workflows-p1-model: Test Contract

Slice P1.1 of Workflows. The phase contract (`testing/feat-workflows-phase-1.md`) is the
authority; this file names what this branch must prove and nothing else.

## Functional Behavior

Phase contract F1 to F7, C2 (no `projects` row), C6, C15 and C16, plus:

- One migration installs `workflows`, `workflow_revisions`, `workflow_runs` and
  `workflow_run_effects` with the Storage section's columns and indexes.
  `LATEST_SCHEMA_VERSION` moves by exactly one.
- `workflows/list_workflows`, `get_workflow`, `save_workflow`, `approve_workflow`,
  `delete_workflow`, `set_workflow_enabled` and `workflow_options` exist on every host
  (daemon dispatch, Tauri command, registry, generated TypeScript) with the phase
  contract's shapes.
- Every mutation publishes `workflows/changed` with `{ "workflowId": "<id>" }`.
- Validation is a pure function over the definition and a context (registered adapters,
  their model catalogs, connector servers, whether the project exists), so every refusal
  is testable without spawning a harness.
- `save_workflow` with no `workflowId` creates; with one, it adds a revision.
  `approve_workflow` approves only the current revision; approving a stale revision id is
  refused. `set_workflow_enabled(true)` on a `project_removed` workflow is refused.
- A workflow with `projectId: null` gets `<data_dir>/workflows/<id>/` on save; delete
  removes it. Deleting a workflow deletes its revisions, runs and effects.
- `src/api.ts` serves the same methods in mock mode with a small fixture set.

## Unit Tests

Phase contract U-M1 to U-M9, U-P1, U-P2. In addition:

- `migration_installs_the_workflow_tables` in `store.rs` tests.
- `approving_a_stale_revision_is_refused` and `re_enabling_a_project_removed_workflow_is_refused` in `workflows.rs`.
- `every_mutation_publishes_workflows_changed` in `workflows.rs` (via `api`).

## Integration / Functional Tests

- The existing gates stay green: `tsgen::checked_in_artifacts_match_the_contract`,
  `params_types_are_named_after_their_method`, the deck's handler-registry and
  command-signature parity tests.
- A daemon dispatch test saves and lists one workflow through `bridged`.

## Smoke Tests

- S-1 from the phase contract (fresh migration from schema 66).
- S-2 from the phase contract (`list_workflows` on a fresh data dir).

## E2E Tests

N/A for this slice; no user journey exists until P1.4.

## Manual / CLI Tests

On a scratch data dir with a short path:

- `bridge exec --json --data-dir <dir> --method workflows/list_workflows` returns `{"workflows":[]}`.
- `bridge exec --json --data-dir <dir> --method workflows/workflow_options` lists Codex and
  OpenCode with `connectorsAllowed: false` and a reason.
