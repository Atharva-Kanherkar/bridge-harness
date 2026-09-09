# Role profiles and adaptive learning

Bridge stores versioned role profiles and typed learning evidence in its local `bridge.db`. The deterministic policy still owns permissions, sandboxing, tool access, provider availability, user pins/exclusions, and request budgets. Learning can rank only candidates that already passed those gates.

Router learning is scoped to `workspace:{id}`. Direct chats are out of the router. Existing rows from before this split keep `legacy:global`; live routing never selects that bucket. Memory jobs never enter the learning router.

Session recall is a different product: FTS5 over that chat's forest, keyed by session id, zero LLM. It is not the helper picker, not account memory, and not a workspace-wide index. See [session-forest.md](./session-forest.md#session-recall).

Account memory is a third product: explicit pins in `memory_records` under `account:local`. See [memory-ledger.md](./memory-ledger.md). Pins never enter the learning router.

Online routing treats missing or stale quota/context as unknown (eligible). Only a live session in the same workspace can mark a harness `QuotaExhausted` or `ContextExhausted`. An old row at `usage_percent=100` does not block a later route.

## Manual and in-app runs

Use **Learning router → Run learning now** for an immediate local run in the current workspace. The same settings panel can enable Bridge's in-app schedule. If Bridge was closed across several intervals, startup performs at most one catch-up and advances `next_run_at` from the current time. A scheduled or CLI wake-up iterates workspaces that have routing outcomes **one at a time** (the durable lease is still global) and never mixes their evidence.

Each run acquires an expiring durable lease, freezes an evidence high-water mark, runs deterministic evaluations, aggregates by task fingerprint/profile/provider/model/effort, and replays a candidate against that workspace's last 5,000 held-out realized outcomes. An outcome whose own result said nothing also queues one bounded model evaluation, executed after the run by whichever host owns the data directory. Reports carry the spend and tokens those runs actually observed, and the Learning router dialog names the execution state rather than claiming there is no executor. A zero spend or token budget still makes the whole learning run an auditable no-op, and a positive one is a hard ceiling: once a run's evaluations have observed it, the rest are settled skipped as evaluation_budget_exhausted rather than executed. Reports compare quality, provider-reported cost per successful task, latency, retries, interventions, and confidence. Missing provider cost remains unknown.

The Learning router dialog is the helper picker (pass, latency, cost). It is not Bridge's memory engine. Provider `/memory` and `/memories` stay on that provider and are not merged here. Saving the panel writes the in-app schedule only when enabled, cadence, or mode actually changed. While the job is already enabled, `next_run_at` is owned by the runner and is not overwritten by a stale dialog snapshot. Rollback restores the predecessor of the live (canary, else active) policy in this workspace — not the latest run's `basePolicyVersion`. The dialog refetches that workspace's learning state when `learning-job-changed` fires.

Learning-policy **promotion** has three modes. Router execution has a separate
`disabled` / `shadow` / `autonomous` control: shadow keeps executing the baseline
while measuring recommendations. Promotion does not itself enable autonomous
routing or grant permissions.

- **Manual** persists a replay-approved recommendation and never promotes it.
- **Ask** requires a separate user approval before one atomic promotion transaction.
- **Automatic** is opt-in and promotes only to a guarded canary. Regression creates and activates a new immutable rollback version based on the predecessor in the same workspace. A first workspace policy has no predecessor; a regressed canary in that case is rolled back and leaves the workspace with no live learned policy.

Cold start, insufficient confidence, duplicate triggers, unavailable candidates, replay regressions, and a zero evaluation budget are visible, auditable no-ops.

Prompt changes are a separate, human-approved configuration operation; learning
does not generate or approve them. See
[approved agent prompt changes](design/agent-prompt-mutation.md).

## Bounded outcome evaluation

Most delegations finish without an automated test, so their outcome lands on one middle confidence constant and every unknown result looks alike to the learner. The bounded evaluator exists to replace that constant where a second opinion is available, and to do nothing else. A verdict is evidence for confidence: it never changes an outcome's success or acceptance state, never grants a permission, and never widens what a policy may promote.

A learning run queues one evaluation per unknown outcome that has an eligible judge. The lifecycle is `not_requested`, `queued`, `running`, `completed`, `failed`, `skipped`. A run leaves `queued` only by being claimed under an expiring lease and leaves `running` only by settling; the first settlement stands. Turning evaluation off for a workspace skips whatever is queued instead of executing it. `routing_policy::load_evidence` reads a `completed` evaluation and nothing else, so a policy cannot be promoted on the strength of a verdict that has not been reached.

The judge runs in a hidden session with an empty tool scope, one turn, no repair, and a wall-clock cap. It never sees a transcript. Its evidence is a capped, SHA-256'd JSON object built from what Bridge and the check executors recorded: the acceptance criteria, per-check statuses, files changed, runtime, retries, override signal, reported cost and tokens, and the bounded artifact ids the deterministic evaluation already stored. What the worker said about its own work is excluded, and so is the acting harness and model name — a self-report and a model identity each move a judge more than most of the real signal does. The acceptance criteria come first, the evidence follows as one JSON object, and the rubric and answer contract come last, so content that breaks out of the data cannot land after the rules.

The rubric asks for a closed set of independent pass/fail criteria rather than a graded score, because judges separate degrees of quality poorly and independent binary questions well. It asks for no explanation beyond one short span quoted per criterion, and for no suggested fix: asking a judge to also propose a repair is the single largest measured regression in its accuracy. Bridge derives `score_bps` from the pass and fail verdicts itself; criteria the model marks as insufficient evidence leave the denominator and cap the recorded confidence in proportion to how much of the rubric was decided.

Neither a malformed answer nor an undecidable one writes a score. A malformed answer settles `failed`, an all-undecidable one settles `skipped`, and both write nothing. Defaulting a broken grader to the worst possible score is a documented way to poison everything downstream that reads one.

Two cautions worth keeping in mind when reading a verdict. Cross-family judging is sound practice and Bridge enforces it, but the size of the self-preference effect is weakly quantified and its direction is not universal — at least one current model has been measured judging its own output more harshly, not more generously. And a score is a surrogate for the outcome, not the outcome: greedy decoding does not reliably reproduce borderline verdicts even at temperature zero, so the sampling parameters a run was given are recorded beside its verdict and the number is a measurement with error. Nothing here fuses verdict-derived confidence into the hard-outcome signal; a design that keeps a predictor trained only on realized outcomes beside one that also consumes verdicts stays open.

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
