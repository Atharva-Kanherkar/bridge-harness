# feat/workflows-multi-step — Test Contract

Locked before implementation. Issue: multi-step workflows with an agent per
step (Phase 2b of the Workflows epic). Linear only: no branches, loops or
parallel steps.

## Definition and protocol

| ID | Expectation |
|----|-------------|
| M1 | `WorkflowStep` gains `servers` (the workflow grants this step uses) and `files` (whether it works in the run's worktree). Both are optional and omitted when unset, so a Phase 1 definition round-trips byte for byte and keeps its digest. `WorkflowRun` gains `steps`, one row per started step: index, agent, model, status, session, summary, failure code, times, tokens and cost. Protocol 1.58 refuses older clients, and 1.58 pairs with itself. |

## Validation and approval

| ID | Expectation |
|----|-------------|
| M2 | A workflow has 1 to 5 steps. Each step's agent, model, effort and instructions are checked as today. With more than one step, each message names the step ("Step 2: ..."); single-step messages are unchanged. |
| M3 | A step's `servers` are workflow grants, each named once. Every grant is used by at least one step. A step's effective servers are its `servers`, or every grant when unset. Its effective files are its `files`, or the workflow's when unset. |
| M4 | A step that works in the worktree uses no connectors. The workflow's `files` is `worktree` exactly when some step works in it. A single-step worktree workflow with grants keeps today's message. |
| M5 | A step with connectors, or with no files, needs an adapter that passes the briefing gate (Claude). A worktree step needs Claude, or Codex, whose launch isolates connectors (M12). OpenCode and the rest are refused with the gate's reason, at save and again at launch for stored definitions. `workflow_options` reports Codex `worktreeAllowed: true` and `connectorsAllowed: false`. |
| M6 | Approval is cleared by: a different agent at any position, a step added or removed, a step that gains a server, or a step that moves into the worktree. All of today's widening rules still hold. Rewording a step or changing its model keeps approval. |

## The run

| ID | Expectation |
|----|-------------|
| M7 | A run starts its steps in order, one chat session per step. Each started step has a `workflow_run_steps` row (migration 97), and the run's `sessionId` is the current step's. Only one step session is active at a time, so a run has at most one approval card waiting. |
| M8 | A step that does not end `done` stops the run, and later steps never start (no session, no step row). The run settles with that step's status and failure code, and its summary begins "Step N of M: ". The cases are `not_done`, a failed turn, no result block, the budget, approval expiry, the provider ending and an unavailable connector. With one step, the summary has no prefix. On success the summary is the last step's. |
| M9 | Each step session runs under a single-step view of the definition: its own grants, the always-allow entries for those servers, its own files and the budget that is left. A tool granted only to step 1 is outside step 2's policy and is never always-allowed there. |
| M10 | From step 2 on, the first turn carries the trigger and events as today, plus the earlier steps' summaries. They are inside a fenced JSON block, labelled as data from another agent and not instructions. A summary containing a fence cannot close that block. |
| M11 | The budget is the run's. Turns, tool calls, output tokens, cost and working time spent by earlier steps are subtracted from the next step's ceilings, and the SDK receives the remainder. A step left with nothing does not start: the run fails `budget_exhausted`. Run tokens and cost are the sum over step sessions, and are unknown if any part is unknown. |
| M12 | A Codex worktree step: the adapter first reads the config Codex reports for the step's folder. It then launches with `enabled=false` for every MCP server, and with `features.apps`, `features.plugins`, `features.remote_plugin` and `features.skill_mcp_dependency_install` false. Before the thread starts, that process's own config must show each server and feature off. After the thread starts, no MCP server may report a tool. A key Bridge cannot write safely, or any failed check, refuses the launch with the reason, and the run fails. (Revised after the live run: per-plugin switches missed account-level remote plugins, which connect seconds after the thread starts, so the plugin features are switched off instead.) |
| M13 | One worktree per run, created at the first worktree step and reused by later worktree steps. Steps without files run in the workflow home, or in a scratch folder for a project workflow. |
| M14 | Recovery and cancellation. A crash between or during steps settles the run `interrupted` at boot, or `lease_expired` on lease expiry. Either way the open step row is closed, and no later step starts. Cancel stops the current step and starts no other. The effect key stays per run: a write repeated with the same arguments in a later step is declined as already run. The run waits for quota when any step's provider is above the reserve. |
| M15 | Phase 1 compatibility: a single-step workflow saves, approves, runs as one step row and reads back unchanged. Runs from before migration 97 still map their session to the run. |

## UI

| ID | Expectation |
|----|-------------|
| U1 | The editor shows the steps. "Add a step" (up to 5) adds one with the first allowed agent. Each later step has Remove. Step 1 keeps today's labels: agent radios, "Workflow model" and "Workflow instructions". Step N uses "Step N agent", "Step N model" and "Step N instructions". |
| U2 | With more than one step, each step shows "Uses" (checkboxes over the granted connectors) and, when the workflow works in a worktree, "Works in the worktree". The worktree checkbox no longer clears connectors when there is more than one step. Each step's agent choices show that step's refusal reason, and Save is disabled while any step is refused. |
| U3 | The run view lists the steps of a multi-step run: number, agent, status and summary, with steps not started shown as such. Choosing a step shows its own transcript and cards; the default is the current or last step. A single-step run looks as it does today. |

## Tests

- `workflows.rs`: M2 to M6, per case.
- `workflow_run.rs`: the step view (M9), hand-off (M10), the budget remainder (M11), step order and stop (M7, M8), recovery (M14) and compatibility (M15).
- `codex_adapter.rs`: M12, against a stand-in app-server. It covers:
  - every key turned off;
  - an unsafe key refused;
  - a server left on refused;
  - a tool still reported refused;
  - no config reply refused.
- Protocol round-trip and handshake (M1).
- Vitest for U1 to U3.
- `bridged` journey E9: a two-step run on the fake SDK. Different models and servers per step; order; the hand-off in step 2's first turn; step 2's policy without step 1's server; one settle; a failed first step stops the run.
- Live, scratch `bridged`: a two-step run with Claude then Codex, in a worktree. Codex reports no MCP tool.
- Gates: `cargo test --workspace`, `bun run build`, vitest, `tsc -b`, clippy ratchet.

## Review fixes

| ID | Expectation |
|----|-------------|
| R1 | The editor never keeps per-step settings that the single-step controls cannot show. When the steps drop to one, the remaining step keeps exactly what it had and loses its own `servers` and `files`. A worktree step keeps the worktree and no connectors. Any other step keeps the grants it used, with their always-allowed tools, and the grants only removed steps used are dropped. Clearing the project returns every step to the workflow's files. `stepsProblem` checks a single step as the daemon does, so Save waits instead of failing. |
| R2 | A step row is written only once the run moved to that step. If the run was cancelled or lost its lease while the step was being set up, `transition` refuses and no step row exists, so no run shows a step still running after it ended. |
