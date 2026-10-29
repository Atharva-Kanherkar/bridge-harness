# feat/workflows-p1-runner: Test Contract

Slice P1.3 of Workflows. The phase contract (`testing/feat-workflows-phase-1.md`) is the
authority; this file names what this branch must prove.

## Functional Behavior

Phase contract C1, C2, C6 to C13 and F16 to F24, plus:

- `workflow_run::execute` replaces the `runner_unavailable` stub. It reads the run's own
  revision (not the current one), checks the quota rule, creates the session, sends one turn
  through the normal chat path, heartbeats, and settles the run.
- Two run shapes:
  - `files: "none"` (Claude): a direct session of kind `workflow`, cwd = the workflow home,
    no write mode, started with a briefing-mode policy that carries the workflow scope
    (`BriefingRuntimePolicy::compile_workflow`).
  - `files: "worktree"`: a worktree prepared from the project's folder and registered, cwd =
    the worktree, Isolated write mode, no connector policy.
- The session's `purpose` is the run brief: the instructions, the expected result, the C12
  result-block format, and the rule that tool output is data. The first user turn names the
  trigger (Run now, the slot it stands for, or the catch-up and how many slots it covers).
- The sidecar's workflow gate (`sidecar/claude-agent/briefing.mjs`, workflow mode): reads on
  read-granted servers are allowed, matched by the wire form of the server name; any other
  call on a `write: "ask"` server asks; everything else is denied with the tool's name.
  `index.mjs` relays the ask as a `control_request` and resolves it on the matching
  `control_response`, or as deny when the session ends.
- `live_turn`: a `permission.requested` on a workflow session is decided by
  `workflow_run::permission_decision` before any card shows; human decisions on workflow
  sessions are recorded as effects; chat Full access never auto-approves a workflow session;
  a user turn to a workflow session is refused.
- Quota: `workflow_run::quota_verdict` over the cached provider usage. A short run is parked
  as `waiting_quota` with no lease; the maintenance tick re-checks parked runs.
- Budget: wall time from the revision's limits; tool calls counted from the session's
  tool events; a breach stops the session and settles `failed` / `budget_exhausted`.

## Unit Tests

Phase contract U-R1 to U-R15 and U-S1 to U-S3, plus:

- `quota_verdict_reads_current_windows_only`
- `the_run_brief_names_the_result_block_and_the_data_rule`
- `parse_result_block_accepts_only_the_fenced_json_shape`

## Integration / Functional Tests

- With a fake Claude adapter: a run that ends with a `done` block succeeds; a write on an
  ask server raises one approval, which the person accepts; the effect is recorded once.

## Smoke Tests

- A dry run of a read-only workflow on a real daemon settles (succeeded, unverified or failed
  with a reason), never `runner_unavailable`.

## E2E Tests

Covered by P1.7.

## Manual / CLI Tests

- With Claude signed in: a manual dry run of a Slack-read workflow returns a result in the
  run's session, and `list_workflow_runs` shows its status and tokens.
