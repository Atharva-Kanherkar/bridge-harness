# feat-session-fork — Test Contract (backend PR, step 1 of 2)

Closes #280 (fork). This contract covers the **backend** PR of the stacked
pair. The frontend PR (fork dialog, per-message affordances, breadcrumbs,
rewind wiring) gets its own contract on the stacked branch above.

## Functional Behavior

New protocol method `sessions/fork_session`:

- Params: `session_id`, `entry_id` (fork point), `title?`, `harness?`,
  `model?`, `worktree_policy` (`"shared"` default | `"new"`).
- Validates:
  - session exists, else typed `invalid` error naming the session id;
  - entry exists and belongs to the session, else typed error;
  - session kind is `orchestrator` or `direct` — worker sessions are
    rejected with an explicit "worker sessions cannot be forked" error;
  - `harness`/`model` overrides, when supplied, are validated against
    known harness names / non-empty model strings.
- Creates a new session that is a **copy of the parent's branch prefix up to
  and including the fork point**:
  - new `sessions` row: fresh UUID id, `parent_session_id` = parent,
    `depth` = parent.depth + 1, `kind` inherited, harness/model inherited
    (or overridden), `status='idle'`, `metric_source='estimated'`,
    `continuation_fidelity='projected_at_boundary'`, title = supplied or
    `Fork of <parent label>`;
  - origin columns on the new row: `fork_parent_entry_id` = fork point,
    `fork_worktree_policy` = the policy used;
  - `session_entries`: every row on `branch_to_leaf(parent, fork_point)`
    copied verbatim (id, kind, payload, sequence, context_visibility,
    token_estimate, created_at) **except** `provider_event_id` cleared to
    NULL — the fork owns no provider-adjacent events yet;
  - `session_heads`: `active_entry_id` = copied fork point id,
    `restoration_mode='checkpoint_restored'`,
    `resume_eligibility='checkpoint_restored'`, `latest_checkpoint_entry_id`
    = nearest checkpoint entry inside the prefix (NULL if none),
    `updated_at` = now;
  - audit: `fork.created` event on the new session naming the parent and
    fork point; `session.forked` event on the parent naming the fork.
- **The parent is byte-for-byte unchanged.** No parent rows are modified.
  Forking a parent that is mid-turn is allowed; the streamed-but-committed
  prefix is whatever the forest holds at call time, and an in-flight turn's
  not-yet-committed writes are simply not part of the copy.
- `worktree_policy="new"`: create a real git worktree (branch
  `bridge/fork/<8-char id prefix>`, sibling path
  `<repo-parent>/<repo-stem>-fork-<prefix>`) and record its path as the new
  session's `cwd`. Git failure rolls the whole fork back — a session is
  never created without its worktree.
- Result: `{ sessionId, snapshot }` where `snapshot` is the new session's
  `SessionForestSnapshot` (so the UI can switch to it immediately).
- Usage and budgets are per-session rows written by the turn loop; the
  fork needs no ledger surgery and must not touch the parent's usage rows.

## Unit Tests (Rust, colocated in `bridge-core/src/sessions.rs`)

- `fork_copies_the_branch_prefix_to_a_new_session` — rows, sequence 1..=n,
  payloads identical, head = fork point, origin columns correct.
- `fork_leaves_the_parent_byte_for_byte_unchanged` — JSON dump of all
  parent tables before/after equals after.
- `fork_of_an_inactive_leaf_is_allowed_and_honest` — fork point on a
  non-active leaf still copies that leaf's prefix; head state stays
  `checkpoint_restored`.
- `fork_rejects_worker_sessions` — typed error, no rows written.
- `fork_rejects_foreign_or_missing_entries` — entry of another session /
  nonexistent entry: typed error, no rows written.
- `fork_clears_provider_event_ids_but_keeps_everything_else` — copied rows
  match except `provider_event_id` is NULL.
- `fork_records_audit_events_on_both_sessions`.
- `fork_with_new_worktree_creates_worktree_and_branch` — temp repo,
  `bridge/fork/` branch exists, cwd records the new path.
- `fork_rolls_back_when_worktree_creation_fails` — stub the git command to
  fail; no session row, no entries, no head.
- `fork_restores_checkpoint_entry_id_when_prefix_contains_one`.

## Integration / Functional Tests

- Regenerated protocol artifacts: `cargo run -p bridge-protocol --bin
  generate-protocol-artifacts` produces a clean diff for the new method's
  params/result types; `src/protocol/generated/protocol.ts` contains
  `sessions/fork_session`; `docs/protocol/schemas/methods.json` lists it.
- `bridgeApi.forkSession` resolves to `sessions/fork_session` (colocated
  `src/api.test.ts` style binding test).
- End-to-end at the `api::fork_session(core, ...)` level: create chat →
  append entries → fork → both snapshots returned and the fork's forest
  matches the prefix.

## Smoke Tests

- `cargo test` (bridge-core suite) green.
- `bun run check` green in the worktree (protocol.ts regenerated).
- `git status` after the protocol regeneration shows only the intended
  generated diffs.

## E2E Tests

N/A for the backend PR — desktop-app UI journeys land with the frontend PR
(step 2 of the stack) and are described in that contract.

## Manual / cURL Tests

N/A — desktop app; the protocol is exercised through Rust integration
tests above. Manual UI pass is specified in the frontend contract.