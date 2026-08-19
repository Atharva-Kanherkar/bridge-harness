# feat/work-contracts-facts — Test Contract

Delivers slice 1 of 7 of the Work epic: durable contracts, migration-safe
storage, and the deterministic offline Facts layer. Locked before
implementation; not edited during it except by a separate commit.

## Scope And Stated Assumptions

- **Store-only means store-only.** `work/get_work_board` reads SQLite and
  nothing else. No git subprocess, no adapter/provider start, no connector
  call, no network. Everything the board needs that is *not* already in SQLite
  is served from a timestamped cache written by some other code path.
- **The Facts layer is the whole product of this slice.** Suggested work is
  contracted (types exist, storage exists, the board carries the fields) but
  never populated here: `tasks` is empty, `latestRun` is `null`, `sources` is
  empty, `usage` is `null`. That is the "placeholder" of the issue's first
  acceptance criterion, and a test pins it so a later slice cannot claim the
  fields were already wired.
- **One new method only.** `work/get_work_board`. The registry is 1:1 with
  `generate_handler![...]` and with `bridged`'s exhaustive dispatch `match`, so
  contracting `work/refresh_work_brief` and friends now would force five
  unimplemented daemon arms. They land with their slices.
- `get_work_board` is contracted **parameterless**. The board is global, the
  same way `state/get_state` is; the absence of params is itself contract and
  the daemon rejects any payload sent to it. Unknown-field rejection is
  therefore exercised on the payload types this slice *does* introduce
  (`WorkSettings` and the fact/task/run DTOs read back out of storage), not on
  a params struct invented to have something to reject.
- **Four fact kinds**, exactly the epic's list: failed completion check,
  blocked worker-queue item awaiting approval, expired-or-actionable approval,
  workspace behind its base branch.
- **Migration 23.** `LATEST_SCHEMA_VERSION` moves 22 → 23. No existing table's
  shape changes, so a database written by this binary stays readable by the
  previous one apart from the new tables.
- Base-divergence observation reuses `git::base_branch_divergence`; this slice
  does not change how divergence is measured, only when it is measured and
  where the answer is kept.
- Out of scope: briefing execution, provider policy, connector registry,
  evidence resolvers, reconciliation, scheduling, and the Work UI.

## Functional Behavior

### The board

`work/get_work_board` returns a `WorkBoard`:

| field | this slice |
| --- | --- |
| `generatedAt` | RFC3339 stamp of the read |
| `facts` | deterministic projections, ordered (below) |
| `tasks` | `[]` — no briefing runner yet |
| `latestRun` | `null` |
| `sources` | `[]` |
| `usage` | `null` |
| `settings` | `WorkSettings` read from `configuration_entries`, or the documented defaults when absent |
| `suggestions` | `WorkSuggestionsState` explaining *why* suggested work is unavailable — `notConfigured` in this slice |

The board is useful with no briefing profile, no provider, and no connector
configured: `facts` is populated and `suggestions.state` names the gap.

### Fact projection

Each fact carries `kind`, `dedupeKey`, `severity`, `title`, `detail`,
`target`, `actionableAt`, `observedAt`, `freshness`, and `action`.

**failed completion check** — one fact per failed `eval_check_runs` row whose
`eval_attempts` row is not `verified`, `waived`, or `superseded`.
- `severity`: `blocking` when the check is required, `attention` otherwise.
- `actionableAt`: the check's `completed_at`, falling back to `started_at`,
  falling back to the attempt's `started_at`.
- `dedupeKey`: `completion-check:{attempt_id}:{check_id}`.
- `action`: `reviewCompletionCheck { sessionId, attemptId, checkId }`. Never
  "clear" — only a new run or an explicit waiver changes a check's verdict.

**blocked worker-queue item** — one fact per `worker_queue` row with
`queue_status='blocked_on_human'`.
- `severity`: `blocking`. Queued work is stopped and only a human restarts it.
- `actionableAt`: `blocked_at`, falling back to `updated_at`.
- `dedupeKey`: `worker-queue:{queue_id}`.
- `action`: `answerApproval { sessionId }` where `sessionId` is the nearest
  ancestor session in `waiting` state (resolved by recursive CTE over
  `sessions.parent_session_id`), falling back to `parent_session_id` when no
  waiting ancestor is recorded. Pointing the action at the queue row itself
  would be useless: the queue row is not what the human answers.

**actionable approval** — one unresolved `session_entries` row of kind
`approval.requested`. Resolved means a later `approval.resolved` entry in the
same session matches it by `requestEventId` (adapter shape, nested under
`data` or top-level) or by `approvalId` (policy shape).
- `severity`: `blocking` once the request is older than
  `WORKER_APPROVAL_TIMEOUT_SECONDS` (expired), `attention` before that.
- `actionableAt`: the entry's `created_at`.
- `dedupeKey`: `approval:{session_id}:{sequence}`.
- `action`: `answerApproval { sessionId, approvalSequence }`.

**workspace behind base** — read from `work_fact_cache`, never from git.
- Emitted only for a cached observation whose `behind` is at or over
  `BaseBranchDivergence::WARN_BEHIND` (20), or whose observation `failed`.
- `severity`: `attention`.
- `actionableAt` / `observedAt`: the cache row's `observed_at`.
- `freshness`: `live` under the staleness window, `stale` over it, `unknown`
  when the observation failed. A missing row emits no fact at all — Bridge has
  not looked, and inventing "up to date" is exactly the failure the epic bans.
- `action`: `refreshWorkspaceBase { sessionId, workspaceId }` when the
  observation is `live`; `refreshBaseObservation { sessionId, workspaceId }`
  when it is `stale` or `unknown`. A stale number is not something to act on —
  it is something to re-measure.

### Dedupe and ordering

- Facts are deduplicated by `dedupeKey`, first occurrence wins.
- Order is total and deterministic: `severity` (blocking, attention, info),
  then oldest `actionableAt` first, then `dedupeKey` ascending. Two facts can
  never compare equal, so the order does not depend on SQLite row order.

### Divergence observation refresh

- `work::refresh_base_divergence(core)` measures every non-archived workspace
  that has a session and writes one `work_fact_cache` row per workspace.
- A successful measurement stores `status='ok'` with the serialized
  `BaseBranchDivergence` and `observed_at = now`.
- A failed measurement (path gone, not a repository) stores `status='failed'`
  with the reason in `detail` and no payload. It does **not** delete or
  overwrite a previous good payload's numbers with zeros.
- It is called from the existing worker-maintenance tick, off the read path,
  and write-through from `api::workspace_base_divergence` /
  `api::refresh_workspace_base` so a user-triggered measurement is not thrown
  away.
- Staleness window: `WORK_FACT_STALE_AFTER_SECONDS` (900s).

### Storage

Migration 23 creates, transactionally:

- `work_brief_runs` — trigger, profile reference, session id, status, limits,
  output digest, parse/error code, usage columns, timestamps.
- `work_brief_sources` — per run and connector instance, with
  eligible/consulted/succeeded/failed/auth-required status.
  `UNIQUE(run_id, connector_instance_id)`.
- `work_evidence` — run-local provenance metadata and digests only.
  `UNIQUE(run_id, evidence_ref)`.
- `work_tasks` — `UNIQUE(fingerprint)`, canonical identity, validated display
  fields, state, evidence digest, miss count, resolution metadata, pin/snooze.
- `work_fact_cache` — `PRIMARY KEY(kind, cache_key)`, `observed_at`, `status`,
  `payload`, `detail`.

No Work table stores a raw connector payload, a credential, or an unbounded
copy of model output; the columns that could are digests and bounded text.

## Unit Tests

### `bridge-protocol` — `messages/work.rs`

- `work_board_round_trips_with_camel_case_wire_names` — every DTO survives a
  JSON round trip and the wire names are camelCase.
- `fact_actions_are_tagged_unions` — each `WorkFactAction` variant serializes
  with its `kind` discriminator and its own fields; an unknown `kind` is
  rejected.
- `work_settings_reject_unknown_fields` — `WorkSettings`,
  `WorkBriefLimits`, and the task/run DTOs refuse a field they do not declare.
- `severity_and_freshness_wire_values_are_snake_case` — enum spellings pinned.
- `absent_options_stay_off_the_wire` — `skip_serializing_if` behaviour pinned
  so the generated TypeScript optionality is honest.

### `bridge-protocol` — registry and generated artifacts

- `every_method_in_the_registry_is_contracted_exactly_once` (existing) covers
  `work/get_work_board`.
- `every_result_is_typed_or_a_documented_exception` (existing) — the new
  method has a typed result, so it must not appear in `DEFERRED_RESULTS`.
- `generated artifacts are current` (existing `tsgen` drift tests) — fails
  until `docs/protocol/schemas/*.json` and `src/protocol/generated/protocol.ts`
  are regenerated.

### `bridge-core` — `work.rs` projection

- `failed_required_check_projects_a_blocking_fact` — and a non-required one
  projects `attention`.
- `verified_waived_and_superseded_attempts_project_nothing`.
- `blocked_queue_item_points_its_action_at_the_waiting_ancestor` — including
  the fallback when no ancestor is `waiting`.
- `resolved_approvals_project_nothing` — for both the `requestEventId` and the
  `approvalId` resolution shapes.
- `an_approval_past_the_deadline_is_blocking_and_a_young_one_is_attention`.
- `divergence_facts_come_from_the_cache_with_their_freshness` — fresh, stale,
  failed, and missing cases; missing projects no fact.
- `a_stale_observation_offers_re_observation_not_a_fast_forward` — action
  mapping is driven by source state.
- `facts_are_ordered_by_severity_then_age_then_key`.
- `duplicate_source_rows_project_one_fact` — dedupe by `dedupeKey`.
- `an_empty_database_projects_an_empty_board` — and every fact kind has an
  explicit empty case.
- `the_board_is_useful_with_no_profile_or_connector` — `settings` defaults,
  `suggestions.state == notConfigured`, facts still present.

### `bridge-core` — divergence refresh

- `a_successful_observation_is_cached_with_its_timestamp`.
- `a_failed_observation_is_recorded_as_failed_and_keeps_no_stale_numbers`.
- `refreshing_twice_updates_in_place` — `PRIMARY KEY(kind, cache_key)` holds.
- `a_user_triggered_divergence_read_writes_through_to_the_cache`.

### `bridge-core` — store-only read path

- `the_board_read_spawns_no_git_process` — instrumented: `git.rs` counts
  every `git` subprocess it starts through a single constructor; the counter
  must not move across a `get_work_board` call, on a database whose workspace
  paths point at directories that are not repositories.
- `the_board_read_starts_no_provider` — the adapter map and runtime session
  map are untouched.
- `the_board_read_path_names_no_io_module` — source-level gate over
  `work.rs`: the projection module references no `git::`, `marketplace::`,
  `Command::new`, or HTTP client symbol.

### `bridge-core` — migration 23

- `migrates_current_schema_fixture_idempotently_and_creates_backup`
  (existing, extended) — the applied-version list becomes `1..=23` and every
  new table exists on the upgraded legacy fixture.
- `work_tables_declare_their_unique_constraints` — inserting a duplicate
  `(run_id, evidence_ref)`, a duplicate `(run_id, connector_instance_id)`, and
  a duplicate task fingerprint each fail.
- `work_tables_cascade_from_their_run` — deleting a `work_brief_runs` row
  removes its sources and evidence.
- `a_failed_migration_23_rolls_back` — the transaction leaves
  `schema_version` at 22 and creates none of the new tables.
- `schema_signature_is_stable_between_fresh_and_upgraded_databases`
  (existing) — a fresh database and an upgraded legacy one agree.

### `bridge-core` — protocol mirror

- `work_board_mirrors_its_protocol_type` — `assert_mirrors::<wire::WorkBoard>`
  over a board with one fact of every kind; enum mirrors get exhaustive
  `match` arms so adding a core variant fails to compile until contracted.

## Integration / Functional Tests

- `bridged` dispatch: `work/get_work_board` returns a board; sending params to
  it fails with `invalid_params`.
- Shell parity tests (existing): the registry and `generate_handler![...]`
  stay 1:1, and the command signature matches the contracted parameterlessness.

## Smoke Tests

- `bun run build` — clean.
- `bun run test` — sidecar tests, `vitest run`, and
  `cargo test --workspace` all green.
- `cargo run -p bridge-protocol --bin generate-protocol-artifacts` leaves the
  working tree clean (proves the checked-in artifacts match the contract).

## E2E Tests

N/A for this slice — there is no UI yet, and the Work screen lands in slice 3.
The board is reachable only through the protocol, which the dispatch tests
cover.

## Manual / cURL Tests

There is no HTTP surface. The equivalent manual check is a JSON-RPC round trip
over the daemon's stdio transport:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin bridge -- \
  exec --json --method work/get_work_board
```

Expected: a `WorkBoard` whose `facts` reflect local state, `tasks` is `[]`,
`latestRun` is `null`, and `suggestions.state` is `"notConfigured"`.

Migration check against a copy of a real database:

```bash
cp ~/Library/Application\ Support/dev.bridge.deck/bridge.db /tmp/bridge-23.db
sqlite3 /tmp/bridge-23.db "SELECT MAX(version) FROM schema_version;"
```

Expected: 22 before, 23 after the app opens it, with a timestamped backup
beside it and every pre-existing row intact.
