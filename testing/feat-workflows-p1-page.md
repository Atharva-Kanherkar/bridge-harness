# feat/workflows-p1-page: Test Contract

Slice P1.4 of Workflows. The phase contract (`testing/feat-workflows-phase-1.md`) is the
authority; this file names what this branch must prove.

## Functional Behavior

Phase contract F25 to F29, plus:

- The sidebar row reads "Workflows" and opens the `workflows` view; the `automations` view
  and every "Scheduled tasks" string in the app are gone. Navigation history knows `workflows`.
- The Workflows page has two sections: Workflows and Native schedules. Native schedules is the
  existing panel, embedded, with all its actions.
- List rows show name, status (active, paused, project gone, needs approval), trigger in
  words, project or "No project", agent and model, next run, last run, a needs-you dot, and
  Run now.
- Detail shows instructions, expected result, trigger (words plus cron and timezone), grants,
  always-allowed tools, budget and the last 20 runs, with Approve, Run now, Dry run,
  Pause or Resume, Edit and Delete (confirmed).
- The editor creates or edits: name; project defaulting to "No project"; schedule presets
  that compile to cron, or custom cron, with a timezone; agent radio list where a harness that
  cannot run this shape is disabled with the gate's reason; model; instructions; expected
  result; a worktree toggle only when a project is chosen, which turns connectors off; per
  server read and "write, asking every time"; time and tool-call limits; notify on success.
  "Approve and save" saves then approves; "Dry run first" saves and starts a dry run. Rust's
  refusal text is shown as is.
- The run view shows status, trigger, tokens, summary and failure code; the session's tool
  calls and messages; a pending card's Allow once, Always allow for this workflow, and
  Decline; Stop for an active run; and Continue in chat, which forks the run's session and
  opens the new chat.
- Settings: pause all, and the quota reserve.
- The page refreshes on `workflows/changed` and `workflows/run_changed`.

## Unit Tests

- V-1 the sidebar says Workflows and never Scheduled tasks
- V-2 a no-project row reads "No project"; the empty state; `harnessRefusal` names the gate's reason
- V-3 the native schedules section renders, with Refresh
- V-4 Continue in chat forks the run's last entry and hands the result to the app
- the editor saves a no-project workflow without choosing a folder
- a decided card offers no answers; an undecided one offers three
- trigger words, preset round trips, relative times (`format.test.ts`)
- navigation history and App source checks updated for `workflows`

## Integration / Functional Tests

- `bun run build` and the full vitest suite.

## Smoke Tests

- In mock mode the page renders list, detail, editor, run view and native schedules, light
  and dark; screenshots go in the PR.

## E2E Tests

Covered by P1.7.

## Manual / CLI Tests

- Open Workflows from the sidebar in mock mode; create, approve, edit, pause and delete a
  workflow; open Native schedules.
