# Delegation policy

Agents may request delegation, but Rust decides whether it runs. Requests cross the adapter boundary as typed envelopes containing role, objective, acceptance criteria, known facts, owned paths, write mode, capability tier, effort, verification steps, and an output contract.

## Decision inputs

The policy engine considers task family, compatible warm workers, requested capability tier, per-turn budget, active leases, owned-path overlap, retry count, and previous outcome. It returns one auditable decision: execute in parent, resume a compatible worker, spawn, queue, reject, or require approval.

Default limits are three workers per user turn, one strong worker, 24 capability units, and one automatic retry. These counters are derived from durable usage-ledger rows keyed by the orchestrator turn. Provider model names are audit data; routing is expressed as `fast`, `standard`, or `strong` capability tiers.

## Write safety

- Read-only workers may run concurrently.
- Write-capable workers must claim paths covered by repository-relative path literals from durable user turns on the parent session's active branch. Bridge grounds those literals against the real workspace tree; `ownedPaths`, `relevantFiles`, and assistant prose cannot authorize themselves.
- A missing or broader-than-proven claim is recorded and routed to user approval before worker reuse, budget consumption, lease acquisition, or process spawn.
- A shared writer requires non-overlapping ownership.
- Overlapping writers are queued FIFO.
- Independent writers receive isolated child worktrees.
- Read-only workers are checked after execution; tracked file changes fail the guard.

The policy gate is deterministic about structure: topology, budgets, tiers, retries, leases, and trusted path provenance. It does not prove that an objective is semantically wise, complete, or appropriately decomposed. User approval remains the boundary when durable user evidence does not authorize the requested write scope.

Workers return a typed result with status, summary, changed files, verification, decisions, risks, remaining work, and suggested next action. Malformed output gets one repair attempt in the same session. Cancellation is terminal: Bridge interrupts the turn, releases its lease, reports cancellation to the parent, and never retries it automatically.
