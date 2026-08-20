# Role profiles and adaptive learning

Bridge stores versioned role profiles and typed learning evidence in its local `bridge.db`. The deterministic policy still owns permissions, sandboxing, tool access, provider availability, user pins/exclusions, and request budgets. Learning can rank only candidates that already passed those gates.

Router learning is scoped to `workspace:{id}`. Direct chats are out of the router. Existing rows from before this split keep `legacy:global`; live routing never selects that bucket. Memory jobs never enter the learning router.

Session recall is a different product: FTS5 over that chat's forest, keyed by session id, zero LLM. It is not the helper picker, not account memory, and not a workspace-wide index. See [session-forest.md](./session-forest.md#session-recall).

Online routing treats missing or stale quota/context as unknown (eligible). Only a live session in the same workspace can mark a harness `QuotaExhausted` or `ContextExhausted`. An old row at `usage_percent=100` does not block a later route.

## Manual and in-app runs

Use **Learning router → Run learning now** for an immediate local run in the current workspace. The same settings panel can enable Bridge's in-app schedule. If Bridge was closed across several intervals, startup performs at most one catch-up and advances `next_run_at` from the current time. A scheduled or CLI wake-up iterates workspaces that have routing outcomes **one at a time** (the durable lease is still global) and never mixes their evidence.

Each run acquires an expiring durable lease, freezes an evidence high-water mark, runs deterministic evaluations, aggregates by task fingerprint/profile/provider/model/effort, and replays a candidate against that workspace's last 5,000 held-out realized outcomes. Bounded model evaluations are currently recorded as deferred work; no model evaluator executes or spends tokens yet. The Learning router dialog labels not-run and deferred work `not_run — no executor`, names deterministic-only runs as such, and disables the spend/token ceiling inputs until an executor exists. Reports therefore show zero evaluator spend/tokens. A zero ceiling still prevents deferred work from being queued at the API; positive ceiling enforcement is reserved for that executor. Reports compare quality, provider-reported cost per successful task, latency, retries, interventions, and confidence. Missing provider cost remains unknown.

The Learning router dialog is the helper picker (pass, latency, cost). It is not Bridge's memory engine. Provider `/memory` and `/memories` stay on that provider and are not merged here. Saving the panel writes the in-app schedule only when enabled, cadence, or mode actually changed. While the job is already enabled, `next_run_at` is owned by the runner and is not overwritten by a stale dialog snapshot. Rollback restores the predecessor of the live (canary, else active) policy in this workspace — not the latest run's `basePolicyVersion`. The dialog refetches that workspace's learning state when `learning-job-changed` fires.

Modes are explicit:

- **Manual** persists a replay-approved recommendation and never promotes it.
- **Ask** requires a separate user approval before one atomic promotion transaction.
- **Automatic** is opt-in and promotes only to a guarded canary. Regression creates and activates a new immutable rollback version based on the predecessor in the same workspace. A first workspace policy has no predecessor; a regressed canary in that case is rolled back and leaves the workspace with no live learned policy.

Cold start, insufficient confidence, duplicate triggers, unavailable candidates, replay regressions, and a zero deferred-evaluation ceiling are visible, auditable no-ops.

## Optional external triggers

External schedulers wake the same local runner; they do not receive the evidence database and do not write the active policy.

Register the trigger from Bridge's settings first. Only one external provider is enabled by default. Disabled, expired, unregistered, or credential-reference-mismatched invocations are audited and do not start a learning run.

```bash
cargo run --manifest-path src-tauri/Cargo.toml -p bridge-client --bin bridge -- \
  learning run \
  --database "/path/to/bridge.db" \
  --trigger codex:daily-learning
```

For Claude Desktop, use `--trigger claude:<registration-id>`. A cloud Routine is experimental and should only wake a future authenticated Bridge endpoint; it must not upload the SQLite/WAL files. If an external adapter needs a secret, put it in the system keychain and pass only a reference such as `--credential-ref keychain:bridge/claude-routine`. Raw bearer values are rejected.

The checked-in [Codex scheduled-task prompt](./prompts/codex-learning-scheduled-task.md) is the source for Bridge's copyable setup instructions and is covered by a native test. Codex Scheduled and Claude scheduling remain user-managed. Bridge does not claim to create, enumerate, or repair provider schedules.

## Evidence and policy history

Routing decisions persist the eligible/excluded catalog snapshot, selected and actual provider/model/effort, profile and policy versions, task fingerprint, repository revision, reason, and override state. Outcomes bind normalized success/unknown, acceptance, retry/edit/intervention, latency, provider cost, token count, and confidence to the decision. Capability-normalized quota cost and provider-reported micro-USD remain separate units. Evaluations store typed bounded metrics and evidence IDs—not concatenated transcripts.

`policy-replay` emits both deterministic safety replay and realized-outcome replay. Realized-outcome replay requires `--workspace` so it cannot mix desks. Promotions and rollbacks are append-only in `routing_policy_promotions`; historical policies and profile versions are never rewritten. Each live policy (active or canary) is unique per `learning_scope`.
