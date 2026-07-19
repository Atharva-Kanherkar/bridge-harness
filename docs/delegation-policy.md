# Delegation policy

Agents may request delegation, but Rust decides whether it runs. Requests cross the adapter boundary as typed envelopes containing role, objective, acceptance criteria, known facts, owned paths, write mode, capability tier, effort, verification steps, and an output contract.

## Decision inputs

The policy engine considers task family, compatible warm workers, requested capability tier, per-turn budget, active leases, owned-path overlap, retry count, and previous outcome. It returns one auditable decision: execute in parent, resume a compatible worker, spawn, queue, reject, or require approval.

Cross-harness continuations are projected rather than native: provider reasoning state cannot move between Codex, Claude, and OpenCode. Policy therefore keeps a cross-harness request queued while its parent has an active turn, releasing it after turn completion or a durable checkpoint, compaction, or worker-result verification boundary. Every session records `native`, `projected_at_boundary`, or `projected_mid_turn`; the last value remains possible under races and is surfaced as degraded rather than hidden.

Human approval pauses dependent queue TTLs. A queued request whose parent or any ancestor is waiting for approval moves to the durable `blocked_on_human` state; it cannot dispatch or expire there. Resolution returns it to `queued` and advances its expiry by the full blocked duration. Cancellation still terminates the dependent request, and block/release transitions are recorded as reason events.

Provider processes are not reattached after a Bridge supervisor crash. Each Codex, Claude, or OpenCode child runs in its own process group, and its leader PID plus OS process identity are persisted on the session. Startup terminates only an exact identity match, marks active sessions recoverably failed, clears the active turn, and routes workers through typed failed-result reconciliation. A PID identity mismatch is never killed. Restart recovery warns that mid-turn worktree changes may be partial and does not invent a checkpoint.

## Cost-and-quality learning router

Every worker route now records the complete harness/model candidate inventory, reason-coded exclusions, conservative prediction, baseline, recommendation, executed candidate, deterministic policy outcome, route status, and eventual worker outcome. Predictions combine explicit tier priors with durable task-family outcomes for pass probability, latency, normalized quota cost, and retry risk. Sparse history remains visibly prior-weighted; it never turns missing data into certainty.

The router starts in `shadow` mode per workspace. Shadow recommendations are measured while the baseline route continues to execute. Autonomous mode cannot be enabled until the workspace has at least 20 completed shadow outcomes with fewer than 5% no-route/manual selections. Users may pin or exclude harnesses and models, but preferences cannot revive a candidate excluded by availability, tools, platform, permissions, quota, context, risk, or the deterministic capability-unit budget. Explicit harness/model selections are retained and labeled as manual overrides.

Learning selects a candidate before the existing policy gate; it does not replace that gate. Owned-path provenance, approval, depth, concurrency, worktree, retry, and budget rules in Rust still decide whether the selected route may spawn, resume, queue, or run at all. Failed worker results become negative outcome labels, not permission to alter safety policy. Escalation only moves to a strictly higher eligible capability tier and is terminal after `strong`.

Replay recorded candidate snapshots without starting a provider or writing to Bridge's database:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --bin router-replay -- /path/to/bridge.db
```

Filter one workspace or test new pins, exclusions, and quality floors against historical decisions:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --bin router-replay -- /path/to/bridge.db --workspace WORKSPACE_ID --preferences candidate-router-preferences.json
```

The report includes recommendation coverage, changed recommendations, shadow alternatives, outcomes, pass rate, latency, normalized cost, retries, human intervention, and policy-violation counts. It is an offline sensitivity report, not a claim of causal model superiority.

Default limits are three workers per user turn, one strong worker, 24 capability units, and one automatic retry. These counters are derived from durable usage-ledger rows keyed by the orchestrator turn. Provider model names are audit data; routing is expressed as `fast`, `standard`, or `strong` capability tiers.

## Offline policy replay

New decision-log entries include a versioned snapshot of every deterministic policy input. Replay the persisted log against the current defaults without starting a provider or writing to the database:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --bin policy-replay -- /path/to/bridge.db
```

To measure a proposed policy, provide a complete JSON `PolicyConfig` using the camel-cased fields in the report:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --bin policy-replay -- /path/to/bridge.db --candidate candidate-policy.json
```

The JSON report separates exact matches, route/reason transitions, route totals, and capability units assessed. Pre-snapshot decisions are counted as `legacySkipped`; malformed or unknown-version records are listed as invalid instead of becoming silent evidence. This is a structural regression and sensitivity benchmark. Quality, realized provider cost, and savings still require outcome labels and billing data, so the replay report deliberately makes none of those claims.

## Write safety

- Read-only workers may run concurrently.
- Write-capable workers must claim paths covered by an explicit `write scope:` declaration in the latest durable user message on the parent session's active branch, or by a policy approval the user accepted for the same parent turn. Historical mentions and arbitrary prose do not grant ambient authority.
- Bridge normalizes repository-relative declarations and grounds them against the real workspace tree. Existing files and directories are eligible; a new file is eligible only when its immediate parent exists. Canonical-path checks reject scopes that escape through symlinks, and component-aware containment prevents wildcard claims from widening a recursive scope. `ownedPaths`, `relevantFiles`, assistant prose, fenced or quoted diagnostics, negated instructions, and unaccepted approval requests cannot authorize themselves.
- A missing or broader-than-proven claim creates one durable, resolvable approval request per turn and scope before worker reuse, budget consumption, lease acquisition, or process spawn. Acceptance records the exact approved scope and active request entry, then re-evaluates the same-turn request; stale-branch, duplicate, session-wide, declined, or cancelled approvals never launch it. If the accepted worker cannot launch or queue, Bridge records and surfaces a retryable failure instead of silently consuming the approval.
- A shared writer requires non-overlapping ownership.
- Overlapping writers are queued FIFO.
- Independent writers receive isolated child worktrees.
- Read-only workers are checked after execution; tracked file changes fail the guard.

The policy gate is deterministic about structure: topology, budgets, tiers, retries, leases, and trusted path provenance. It does not prove that an objective is semantically wise, complete, or appropriately decomposed. User approval remains the boundary when durable user evidence does not authorize the requested write scope.

Workers return a typed result with status, summary, changed files, verification, decisions, risks, remaining work, and suggested next action. Malformed output gets one repair attempt in the same session. Cancellation is terminal: Bridge interrupts the turn, releases its lease, reports cancellation to the parent, and never retries it automatically.

## Completion proof

A completed implementation worker opens a private, durable completion contract from its typed acceptance criteria. The contract, deterministic eval plan, check runs, findings, waivers, and proof bundle live in SQLite; exporting or committing a Markdown projection is optional. This keeps the useful review discipline without adding a contract file to every change.

Bridge runs implementation and verification sequentially. Deterministic build, test, lint, and repository commands become required gates and close only from exact typed command results returned by a verification worker running in the implementation worktree. Model scrutiny and user-journey checks run at meaningful completion boundaries. An active completion gate hard-excludes the implementation harness family from verification routing even while the learning router is in shadow mode. A skill or plugin can register a verifier manifest describing triggers, checks, required tools, cross-family requirements, and evidence; eligible matching manifests are added to the plan, but they cannot grant themselves capabilities or mint passing evidence.

Every attempt records the exact implementation worktree path, Git HEAD, and dirty-tree digest. A revision change supersedes the attempt instead of reusing stale evidence. Required failures and skips remain visible in the inline proof card. `verified`, `changes requested`, and `verified with waiver` are distinct states; a waiver is scoped to named unresolved checks and the exact revision.

The synthetic measurement fixture in `testing/fixtures/completion-benchmark-v1.json` compares proof-gated completion with a baseline that accepts worker “done” claims. Its regression test protects the report calculations and threshold wiring; it is not empirical evidence that the product improves quality or cost. Production claims must come from persisted proof and outcome data.

Each accepted worker result is a canonical `worker.result` entry on the parent's active session branch. Its entry ID is the evidence ID. New sibling delegations include up to the 16 most recent active-branch evidence records by default; an orchestrator may select an ordered subset by ID, but Bridge resolves the exact typed payload from SQLite and rejects missing, foreign-session, abandoned-branch, duplicate, or malformed references before provider startup. The parent runtime receives only a routing notice with the evidence ID, status, and summary. Orchestrator prose is not the record, and raw worker transcripts never enter a context packet.
