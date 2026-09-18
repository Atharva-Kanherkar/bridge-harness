# Delegation policy

Agents may request delegation, but Rust decides whether it runs. Requests cross the adapter boundary as typed envelopes containing role, objective, acceptance criteria, known facts, owned paths, write mode, capability tier, effort, verification steps, and an output contract.

## Decision inputs

The policy engine considers task family, compatible warm workers, requested capability tier, per-turn budget, active leases, owned-path overlap, retry count, and previous outcome. It returns one auditable decision: execute in parent, resume a compatible worker, spawn, queue, reject, or require approval.

Cross-harness continuations are projected rather than native: provider reasoning state cannot move between Codex, Claude, and OpenCode. Policy therefore keeps a cross-harness request queued while its parent has an active turn, releasing it after turn completion or a durable checkpoint, compaction, or worker-result verification boundary. Every session records `native`, `projected_at_boundary`, or `projected_mid_turn`; the last value remains possible under races and is surfaced as degraded rather than hidden.

Human approval pauses dependent queue TTLs. A queued request whose parent or any ancestor is waiting for approval moves to the durable `blocked_on_human` state; it cannot dispatch or expire there. Resolution returns it to `queued` and advances its expiry by the full blocked duration. Cancellation still terminates the dependent request, and block/release transitions are recorded as reason events.

An adapter descriptor declares which sandbox modes its harness can actually start in, including transport constraints. The router excludes an incompatible harness with `PermissionCeiling` before any reservation, so a route guaranteed to fail at adapter startup never creates a worker session; autonomous routing picks a compatible harness instead, and a pinned or manual incompatible route returns an actionable incompatibility naming the harness, the sandbox mode, and the compatible alternatives. OpenCode declares no read-only support because its localhost HTTP transport cannot run inside the offline read-only sandbox; the adapter's own fail-closed guard stays in place as defense in depth.

### Workers blocked on a human

A background worker's in-session approval card renders on the *worker's* conversation, which is normally not the one on screen. Bridge therefore mirrors it onto the parent conversation with the worker label, objective, command, cwd, and owned-path scope, and tells the parent the child is blocked rather than failed. Resolving the approval from any surface updates the parent's mirrored row and the worker lifecycle together.

`waiting` is excluded from the stall watchdog because it is legitimately idle, so it has its own deadline (`WORKER_APPROVAL_TIMEOUT_SECONDS`, 30 minutes). Past it the worker is resolved to a terminal `blocked` typed result naming the unanswered approval, which releases the parent. Entering and leaving `waiting` always moves the durable `waiting_since` stamp, so a resolved approval cannot leave an expired-looking timestamp behind.

### Mid-run visibility

A worker is not a black box between spawn and its typed result. Every routing notice Bridge sends a parent carries a `fleet` digest — per live child: lifecycle, task family, retry count, the one-line `progress_summary` derived from the child's own event stream, and any waiting reason. On demand, the orchestrator emits one fenced `bridge-peek` block (`{}` for all children, `{"sessionId":"…"}` for one) and Bridge replies with a `bridge-worker-activity` digest that adds each worker's most recent durable tool calls and messages, head-truncated and capped (`PEEK_MAX_ENTRIES`).

The digest is host-built from `worker_runtime` and `session_entries`; the model never sees, and must never request, a raw worker transcript. Digest text is evidence about the worker, not instructions to the parent. The same data feeds the user's side: Mission Control tiles show `progress_summary` and waiting reasons, and the worker detail view replays the child's durable event log merged with the live stream.

## Stale base branches

`RepositoryDivergence` compares a conversation entry's saved local stamp with the current local tree; both sides can be months behind the default branch and still read "aligned". A separate check measures the workspace against the best available fetched ref for its upstream or default branch — tracked upstream first, then `origin/HEAD`, then a conventional local default — and reports ahead/behind counts, the compared ref's own age, and whether a fetch was attempted and succeeded, so an offline stale ref is never presented as current truth.

The check runs at workspace open (where a fetch is affordable, off the calling thread) and again before the first write delegation of a turn (no fetch: a delegation must not wait on the network). Past the warning threshold both the user and the orchestrator context are told the counts, and the orchestrator is instructed to offer the choice rather than act. The refresh action is a strict fast-forward: it refuses a dirty tree, an active session, and any local history the base does not contain, so a background warning can never discard work. Warnings are deduplicated per session per base revision.

Completion attempts additionally record the base ref, base commit, worker branch, and worker session id, so proof is bound to what changed *and what it changed from* rather than a bare HEAD string.

Provider processes are not reattached after a Bridge supervisor crash. Each Codex, Claude, or OpenCode child runs in its own process group, and its leader PID plus OS process identity are persisted on the session. Startup terminates only an exact identity match, marks active sessions recoverably failed, clears the active turn, and routes workers through typed failed-result reconciliation. A PID identity mismatch is never killed. Restart recovery warns that mid-turn worktree changes may be partial and does not invent a checkpoint.

## Cost-and-quality learning router

Every worker route now records the complete harness/model candidate inventory, reason-coded exclusions, conservative prediction, baseline, recommendation, executed candidate, deterministic policy outcome, route status, and eventual worker outcome. Predictions combine explicit tier priors with durable task-family outcomes for pass probability, latency, normalized quota cost, and retry risk. Sparse history remains visibly prior-weighted; it never turns missing data into certainty.

The router starts in `shadow` mode per workspace. Shadow recommendations are measured while the baseline route continues to execute. Autonomous mode cannot be enabled until the workspace has at least 20 completed shadow outcomes with fewer than 5% no-route/manual selections. Users may pin or exclude harnesses and models, but preferences cannot revive a candidate excluded by availability, tools, platform, permissions, quota, context, risk, or the deterministic capability-unit budget. Explicit harness/model selections are retained and labeled as manual overrides.

Quota and context exclusions come only from **live** sessions in that workspace (`working`, `waiting`, `starting`, `checkpointing`, `resuming`, `warm`, `restored`, and `ended_at` still null). An ended or ready session that last reported `usage_percent=100` is unknown, which stays eligible — it must not permanently mark the harness `QuotaExhausted` or `ContextExhausted`.

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
cargo run --manifest-path src-tauri/Cargo.toml --bin policy-replay -- /path/to/bridge.db --workspace WORKSPACE_ID
```

To measure a proposed policy, provide a complete JSON `PolicyConfig` using the camel-cased fields in the report:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --bin policy-replay -- /path/to/bridge.db --workspace WORKSPACE_ID --candidate candidate-policy.json
```

The JSON report separates exact matches, route/reason transitions, route totals, and capability units assessed. Pre-snapshot decisions are counted as `legacySkipped`; malformed or unknown-version records are listed as invalid instead of becoming silent evidence. This is a structural regression and sensitivity benchmark. Quality, realized provider cost, and savings still require outcome labels and billing data, so the replay report deliberately makes none of those claims.

## Authorizing a write scope

There are exactly two ways a write-capable delegation becomes authorized, and no third:

1. **The user declares the scope.** A line of the form `Write scope: src/**, docs/**` in the user's message authorizes those paths for that turn. The syntax is exact: the line must start with `write scope:` (case-insensitive), outside code fences and block quotes, and the paths are comma- or semicolon-separated and repository-relative.
2. **The user accepts an approval card.** When no declaration covers the requested paths, the policy raises one approval card per turn and scope. Accepting it authorizes exactly the paths shown.

Anything else — assistant-proposed `ownedPaths`, `relevantFiles`, repository exploration, ordinary path mentions in prose — is *not* authorization. Read-only workers are exempt because they cannot write.

The practical consequence is that a cold-start "implement X" request from a user who has not used the `Write scope:` syntax will raise an approval card. **That is the designed state, not a failure.** Approval-pending is modelled as its own outcome (`AwaitingApproval`) end to end:

- The parent is told `bridge-worker-launch-awaiting-approval` with the delegation identity, the machine-readable `RouteReason`, and a remediation sentence. It is instructed to stop emitting work for that objective and *not* to treat the child as terminal. No `delegation.rejected` entry is written and the "Worker failed to start" card never renders.
- On acceptance the launch proceeds and the parent receives `bridge-worker-launch-approved` carrying the launched child session id (or the queued state), so it re-adopts the child.
- On decline or cancel the parent receives exactly one terminal `bridge-worker-launch-declined` notice.
- Both the parent notice and the approval card carry the structured reason (for example `owned_path_provenance_required`) and its remediation, so neither the user nor the orchestrator has to guess the cause. UI copy is a projection of that data, never the API.

## Write safety

- Read-only workers may run concurrently.
- Write-capable workers must claim paths covered by an explicit `write scope:` declaration in the latest durable user message on the parent session's active branch, or by a policy approval the user accepted for the same parent turn. Historical mentions and arbitrary prose do not grant ambient authority.
- Bridge normalizes repository-relative declarations and grounds them against the real workspace tree. Existing files and directories are eligible; a new file is eligible only when its immediate parent exists. Canonical-path checks reject scopes that escape through symlinks, and component-aware containment prevents wildcard claims from widening a recursive scope. `ownedPaths`, `relevantFiles`, assistant prose, fenced or quoted diagnostics, negated instructions, and unaccepted approval requests cannot authorize themselves.
- A missing or broader-than-proven claim creates one durable, resolvable approval request per turn and scope before worker reuse, budget consumption, lease acquisition, or process spawn. Acceptance records the exact approved scope and active request entry, then re-evaluates the same-turn request; stale-branch, duplicate, session-wide, declined, or cancelled approvals never launch it. If the accepted worker cannot launch or queue, Bridge records and surfaces a retryable failure instead of silently consuming the approval.
- A shared writer requires non-overlapping ownership.
- Overlapping writers are queued FIFO.
- `write mode: isolated` **always** creates a child worktree, including for a lone writer. Isolation is a promise about where writes land, not an optimization that applies only when a second writer happens to be active.
- Every non-read-only worker gets a durable repository binding at launch recording the exact checkout it runs in and that checkout's base revision, so `worktree_path` is never null and a later claim can always be checked against a known base.
- Read-only workers are checked after execution; tracked file changes fail the guard.

## Read-only worker credentials

A read-only worker runs with a redirected `CLAUDE_CONFIG_DIR`, and Claude Code scopes its credential lookup to that directory, so the sidecar cannot see the user's own sign-in. Interactive sessions never take this path. Bridge hands the worker a token according to the Claude harness setting `advanced.workerCredentialSource` (Settings → Harnesses → Claude Code):

- `auto` (default): `CLAUDE_CODE_OAUTH_TOKEN` from Bridge's environment, else the `claude` CLI's macOS Keychain entry. The Keychain is read through `/usr/bin/security`, so macOS may ask once to allow that helper. The entry's modification date is checked without any grant, and the secret is re-read only when the CLI rewrote the entry, so an unchanged token never prompts twice.
- `environment`: the variable only, typically from `claude setup-token`. The Keychain is never consulted. A missing variable fails the launch with a message that names the command and the setting.
- `none`: nothing is injected.

The setting names a source, never a credential. Secret-looking fields in the Claude advanced configuration are refused on save, and a worker that starts without a credential reports it in its startup diagnostics instead of failing opaquely on its first model call.

## Adopting isolated worker output

Verifying a child worktree proves nothing about the user's task checkout. Isolated output therefore has an explicit, durable lifecycle:

```
in_place                                shared/full writer — nothing to adopt
pending_adoption ─┬─ adopted            fast-forward merge into the task worktree
                  └─ discarded          thrown away on purpose
empty                                   isolated writer that changed nothing
```

A parent session cannot become ready while any child sits in `pending_adoption`, so a "verified" change can no longer leave the workspace untouched. Adoption refuses to run against a dirty or active task worktree and refuses a non-fast-forward, leaving the work pending rather than losing it. A child worktree is removed only once its row is terminal, and a dirty one is retained rather than force-removed. Restart recovery reconciles pending rows whose worktree no longer exists, so a missing directory cannot block a parent forever.

## Repository evidence for worker claims

A write-mode worker's typed result is checked against Git before it becomes canonical. Bridge derives HEAD, branch, the commits since the recorded base, committed and dirty paths, and a diffstat from the worker's bound checkout, then:

- Replaces `filesChanged` with the derived path list. The completion planner selects required checks from that field, so a worker cannot suppress a build or test by omitting a path, nor invent irrelevant ones.
- Downgrades a `completed` claim to `blocked` when the repository shows no change at all behind it, or when it wrote outside its owned-path lease, with an explicit evidence-mismatch reason.
- Records claimed-but-absent and changed-but-unreported paths as risks.
- Carries the derived revision, branch, dirty state, and diffstat into the parent-facing result and routing notice.

The policy gate is deterministic about structure: topology, budgets, tiers, retries, leases, and trusted path provenance. It does not prove that an objective is semantically wise, complete, or appropriately decomposed. User approval remains the boundary when durable user evidence does not authorize the requested write scope.

Workers return a typed result with status, summary, changed files, verification, decisions, risks, remaining work, and suggested next action. Malformed output gets one repair attempt in the same session. Cancellation is terminal: Bridge interrupts the turn, releases its lease, reports cancellation to the parent, and never retries it automatically.

## Completion proof

A completed implementation worker opens a private, durable completion contract from its typed acceptance criteria. The contract, deterministic eval plan, check runs, findings, waivers, and proof bundle live in SQLite; exporting or committing a Markdown projection is optional. This keeps the useful review discipline without adding a contract file to every change.

Bridge runs implementation and verification sequentially. Deterministic build, test, lint, and repository commands become required gates, and Bridge itself executes them: a core check runner takes one pending `bridge.shell` check at a time, runs the planned command inside the attempt's exact `repository_path` with no shell, captures bounded stdout/stderr and the exit status, digests that real output, and records the verdict. Commands are allowlisted by program name and refused — recorded `blocked`, with the reason — if they name an unlisted program or carry shell metacharacters.

A verification worker's self-reported `tests[]` can no longer close a `bridge.shell` check: `record_check` rejects `bridge.worker_result` evidence for one, because hashing the JSON that arrived proves only which message was received. Workers settle semantic checks; the runner settles shell checks.

`verifying` has a deadline (`COMPLETION_VERIFY_TIMEOUT_SECONDS`, 45 minutes). Past it, every required check that never reached a terminal state is recorded `blocked` with an explicit reason — naming, for a semantic check, that no eligible verifier claimed it — the attempt fails terminally, and the parent is released so it can report which checks never ran. That escalation is marked on the attempt, which is how a timed-out gate is distinguished from a gate Bridge could not build at all: the latter still fails closed and requires an explicit human waiver.

Model scrutiny and user-journey checks run at meaningful completion boundaries. An active completion gate hard-excludes the implementation harness family from verification routing even while the learning router is in shadow mode. A skill or plugin can register a verifier manifest describing triggers, checks, required tools, cross-family requirements, and evidence; eligible matching manifests are added to the plan, but they cannot grant themselves capabilities or mint passing evidence.

Every attempt records the exact implementation worktree path, Git HEAD, and dirty-tree digest. A revision change supersedes the attempt instead of reusing stale evidence. Required failures and skips remain visible in the inline proof card. `verified`, `changes requested`, and `verified with waiver` are distinct states; a waiver is scoped to named unresolved checks and the exact revision.

The synthetic measurement fixture in `testing/fixtures/completion-benchmark-v1.json` compares proof-gated completion with a baseline that accepts worker “done” claims. Its regression test protects the report calculations and threshold wiring; it is not empirical evidence that the product improves quality or cost. Production claims must come from persisted proof and outcome data.

Each accepted worker result is a canonical `worker.result` entry on the parent's active session branch. Its entry ID is the evidence ID. New sibling delegations include up to the 16 most recent active-branch evidence records by default; an orchestrator may select an ordered subset by ID, but Bridge resolves the exact typed payload from SQLite and rejects missing, foreign-session, abandoned-branch, duplicate, or malformed references before provider startup. The parent runtime receives only a routing notice with the evidence ID, status, and summary. Orchestrator prose is not the record, and raw worker transcripts never enter a context packet.
