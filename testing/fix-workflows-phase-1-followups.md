# fix/workflows-phase-1-followups — Test Contract

Locked before implementation. Completes the runtime follow-ups to the Phase 1
contract, especially C7, F20, F22a and F23. Phase 2 is out of scope.

## Functional Behavior

- A Claude workflow with `files: worktree` retains Isolated file editing and
  shell tools and user skills/settings, but has no MCP servers or plugins.
  Every `mcp__` call is denied before user allow rules can admit it, including
  native subagent calls. Native Task/Agent delegation is refused in worktree
  runs, so child work cannot evade accounting. Codex/OpenCode worktree
  workflows are refused until their adapters can isolate MCP authority,
  including already saved definitions at launch. Ordinary chats and connector
  workflows keep their existing permissions. No public protocol or database migration is needed.
- Each of the five ceilings stops an active run as `failed` with
  `failureCode: budget_exhausted`: working wall time, assistant messages after
  the run baseline, tool calls after the baseline, ledger output tokens, and
  ledger cost in micro-USD. Message/tool ceilings allow exactly N and fail
  only above N; completion on N succeeds and the Nth approval is retained.
  Token/cost ceilings stop at equality. Optional token and cost ceilings that are absent do not stop the run. Input tokens do not
  spend the output-token ceiling; another session's usage does not count.
- A completed turn cannot bypass a reached token/cost ceiling or an exceeded
  message/tool limit. Claude message-start/delta and assistant usage reach the
  ledger before a final result. Repeated or stale snapshots never double-count
  or rewind usage. A final result replaces provisional rows once; empty crash
  results preserve observed usage. Live costs use existing model pricing and
  stay unknown without rates; the SDK also receives the native cost budget.
  Budget monitoring uses the provider's usage reporting granularity.
  Approval waiting pauses working wall time, remains bounded separately at
  the existing 24 hours, and cancellation still stops the underlying session.
- Terminal outcomes sum the run session's ledger input/output tokens and cost.
  Any missing cost keeps the total unknown (`null`); a reported zero remains
  zero. Known partial costs remain a lower bound for budget enforcement. All
  settlement paths share one indexed aggregate. Failed and cancelled runs retain whatever usage was recorded before settlement.
- Rechecking quota leases a `waiting_quota` run without changing that status.
  Parking releases the lease, and repeated blocked checks publish nothing.
  There is one notification on entering quota wait and one on leaving it.
  Maintenance obtains at most one provider snapshot for its claimed batch,
  rather than one per parked run. Crashed leases still expire safely, and
  paused workflows do not restart quota-waiting runs. Disabling a workflow
  (including project removal) cancels parked quota runs with a reason, freeing
  the active slot. Re-enabling never revives the old occasion. Quota snapshot,
  settings or revision errors after claiming cannot orphan the batch; a bad
  revision fails only its own run. Accounting errors fall back to terminal
  runner_error settlement rather than leaving a run leased.
- The existing 24-hour unanswered-approval expiry stays in force unless the
  owner chooses another value. Remote stack branches are preserved unless
  the owner explicitly requests their deletion. These owner choices and
  release-only checks remain visible in the PR if unresolved.

## Unit Tests

- `workflow_run` tests: worktree authority and Isolated mode; each budget
  independently; equality and below-limit boundaries; absent optional limits;
  output vs input accounting; unrelated-session exclusion; completion at a
  token/cost ceiling and message/tool completion at N vs N+1; mixed-null,
  zero and summed costs across success/cancel/reclaim; query-plan index use;
  settlement fallback when ledger reads fail; streaming usage snapshots and
  final reconciliation; active provider shutdown with pre-result usage.
- Existing `workflow_run` tests for approval expiry, paused working time,
  duplicate writes, dry runs, ended approvals and cancellation still pass.
- `workflow_trigger` tests: three blocked quota checks publish one wait event;
  departure publishes one event; leasing does not change visible status;
  stale ownership cannot write; parked runs do not expire while unleased;
  a crash while rechecking remains recoverable; pause holds queued work;
  disable/project removal settles quota waits; malformed revision/settings
  do not abandon other committed claims or suppress notifications.
- Claude sidecar tests in `test/workflow.mjs`: use a temporary settings file
  allowing MCP writes and Bash test commands; the fixture actually loads
  selected settings sources and applies allow rules before canUseTool.
  Verify that MCP writes would execute with the hook removed, while the
  authoritative hook denies them even on a native subagent. Task/Agent are
  refused, while file editing and a Bash test command actually execute.

## Integration / Functional Tests

- Run the daemon `workflows` integration target: existing no-project connector
  journey, project worktree journey, and scheduled catch-up journey pass.
  A live sidecar that never emits a result is stopped separately by output
  tokens, estimated cost and cancellation, retaining its pre-result usage.
- Bind session entries and ledger rows to a claimed run in the Rust tests;
  exercise the real wait/settlement path and inspect persisted run status,
  `failureCode`, `tokens` and `costMicrousd`.
- Subscribe to core events and count quota transition notifications across
  repeated maintenance checks using deterministic provider snapshots.
- `bun run build` and `bun run test` must finish successfully before the PR.

## Smoke Tests

- The staged and bundled Claude sidecar includes every local `.mjs` module,
  including the workflow ask relay and trust reader. Both fresh staging and
  reuse after missing modules are tested by launching the real staged sidecar
  in catalog mode against a local SDK stub, without an account or model call.
  The release gate rejects any missing local module.

- Build a macOS release package and run the packaged-app smoke harness when
  its prerequisites are available. Record exact outcome and build commit.
- N/A — no deployed service or HTTP health endpoint is changed by this PR.

## E2E Tests

- Automated: the daemon workflow journeys above exercise the runner, sidecar,
  approvals, isolated worktree and result persistence with local fixtures.
- Release-app journeys M1–M4 from `feat-workflows-phase-1.md` require the
  desktop UI, light and dark screenshots, real connector/account availability,
  quota pressure, and closing/sleeping the desktop across schedule slots.
  Perform the journeys that can be verified safely on a disposable profile;
  document exact remaining steps rather than claim unperformed checks passed.
  Leave the issue open until release-only acceptance and owner choices finish.

## Manual / cURL Tests

No external cURL requests are needed. Reproducible local commands:

```sh
node --test sidecar/claude-agent/test/workflow.mjs
cargo test --manifest-path src-tauri/Cargo.toml -p bridge-core workflow_
cargo test --manifest-path src-tauri/Cargo.toml -p bridged --test workflows
bun run build
bun run test
```

Manual release checklist (record as performed or explicitly not performed):

1. M1: create/draft a no-project connector workflow, dry run, approve, run,
   answer the write card, open the inbox result, continue in a normal chat.
2. M2: run a project workflow that edits a file; verify its new worktree and
   that the original checkout is untouched.
3. M3: close the desktop window over a scheduled slot; reopen to one run.
   Sleep across two slots and verify one labelled catch-up.
4. M4: use quota reserve 90 with a pressured account; a workflow waits while
   an ordinary chat can still send. Capture light and dark views.
