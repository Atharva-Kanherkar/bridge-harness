# Delegation policy

Agents may request delegation, but Rust decides whether it runs. Requests cross the adapter boundary as typed envelopes containing role, objective, acceptance criteria, known facts, owned paths, write mode, capability tier, effort, verification steps, and an output contract.

## Decision inputs

The policy engine considers task family, compatible warm workers, requested capability tier, per-turn budget, active leases, owned-path overlap, retry count, and previous outcome. It returns one auditable decision: execute in parent, resume a compatible worker, spawn, queue, reject, or require approval.

Cross-harness continuations are projected rather than native: provider reasoning state cannot move between Codex and Claude. Policy therefore keeps a cross-harness request queued while its parent has an active turn, releasing it after turn completion or a durable checkpoint, compaction, or worker-result verification boundary. Every session records `native`, `projected_at_boundary`, or `projected_mid_turn`; the last value remains possible under races and is surfaced as degraded rather than hidden.

Human approval pauses dependent queue TTLs. A queued request whose parent or any ancestor is waiting for approval moves to the durable `blocked_on_human` state; it cannot dispatch or expire there. Resolution returns it to `queued` and advances its expiry by the full blocked duration. Cancellation still terminates the dependent request, and block/release transitions are recorded as reason events.

Provider processes are not reattached after a Bridge supervisor crash. Each Codex/Claude child runs in its own process group, and its leader PID plus OS process identity are persisted on the session. Startup terminates only an exact identity match, marks active sessions recoverably failed, clears the active turn, and routes workers through typed failed-result reconciliation. A PID identity mismatch is never killed. Restart recovery warns that mid-turn worktree changes may be partial and does not invent a checkpoint.

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

Each accepted worker result is a canonical `worker.result` entry on the parent's active session branch. Its entry ID is the evidence ID. New sibling delegations include up to the 16 most recent active-branch evidence records by default; an orchestrator may select an ordered subset by ID, but Bridge resolves the exact typed payload from SQLite and rejects missing, foreign-session, abandoned-branch, duplicate, or malformed references before provider startup. The parent runtime receives only a routing notice with the evidence ID, status, and summary. Orchestrator prose is not the record, and raw worker transcripts never enter a context packet.
