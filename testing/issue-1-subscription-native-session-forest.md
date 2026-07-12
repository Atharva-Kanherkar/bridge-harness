# Issue 1 — Subscription-Native Session Forest Test Contract

This contract locks the definition of done for GitHub issue #1 and subissues #2–#13. Each subissue also receives its own branch-specific contract before implementation.

## Functional Behavior

- Persistence uses ordered, transactional, versioned migrations with a pre-migration backup, foreign keys, single-writer access, and a production session-forest path.
- Conversation history is an immutable entry tree: rewind changes the active leaf, later appends fork history, and no conversation action silently rewinds Git state.
- Active-branch projection is deterministic, compaction preserves raw history, and each resumable agent owns its checkpoint.
- Restoration is accurately labeled as `hot`, `native`, `checkpoint_restored`, or `fresh`; failed native resume is recorded and never mislabeled.
- Every worker spawn/resume/queue/reject passes through deterministic Rust policy with per-turn budgets, typed reason codes, leases, path ownership, and capability tiers (`fast`, `standard`, `strong`).
- Delegation requests and worker results are typed and versioned; default topology is flat; cancellation is terminal; unstructured output gets one same-session repair turn and an explicit fallback.
- Read-only/write permissions are enforced by provider sandbox configuration; overlapping writers never run concurrently and disjoint writers use isolated worktrees.
- Compaction is controller-triggered, immutable, repairable, and skipped for one-shot workers with valid typed results.
- UI exposes the active branch, restoration mode, context pressure, budget, worker role/ownership/state, and inspectable policy/lifecycle reasons.
- No direct OpenAI or Anthropic API credential or network call is introduced; intelligence remains in locally authenticated Codex and Claude Code harnesses.
- Legacy `agent_events`, old delegation counters, and model-branded routing semantics are removed only in Phase 6 after production reads migrate to the forest.

## Unit Tests

- Rust tests cover migrations, append-only history, deterministic traversal/projection, branching, corruption detection, typed envelopes, policy decisions, budgets, lifecycle, resume fallback, compaction, permission mapping, path overlap, and worktree safety.
- TypeScript/Vitest tests cover active-branch conversation projection, stable identity, special cards, restoration badges, queues, and conflict explanations.
- Every subissue-specific test named in #2–#13 exists and passes before its PR is ready.

## Integration / Functional Tests

- Fixture databases from every prior schema migrate safely and idempotently.
- Codex and Claude hot/native/checkpoint restoration paths are exercised; live harness tests may be explicitly ignored in default CI but must have a documented runnable path and be run where the installed harness supports them.
- Restart, cancellation, worker reuse, queue dispatch, compaction failure, worktree isolation/integration, and SQLite-only UI replay are exercised.
- Full commands: `bun run test`, `bun run check`, and `bun run build` all pass on the final integrated branch.

## Smoke Tests

- A fresh application database opens and exposes an empty usable state.
- An existing database opens, migrates, and replays its existing session UI unchanged during compatibility phases.
- The Tauri backend compiles and all frontend production assets build.

## E2E Tests

1. A long-running orchestrator survives at least three compactions without losing decisions.
2. A compatible standard worker resumes for a related fix without full repository rediscovery.
3. A one-shot test worker returns a typed result and terminates without compaction.
4. A strong planner terminates only after its structured decision/checkpoint is attached to the parent.
5. Restart distinguishes native resume from checkpoint restoration.
6. Conversation rewind forks history without deleting entries or changing files.
7. Overlapping writers never run concurrently.
8. No direct external LLM API credential or call exists.
9. Model intelligence is supplied only by local Codex/Claude harness processes.
10. Every worker policy and lifecycle decision is inspectable.

## Manual / cURL Tests

- N/A for HTTP cURL: Bridge is a local Tauri application, not an HTTP service.
- Run the final app with `bun run tauri dev`; exercise branch/rewind, compact, restoration, queued conflict, worker drill-down, and cancellation flows against the acceptance scenarios above.
- Inspect the migrated SQLite schema and audit events to verify restoration labels, policy reasons, leases, usage, and immutable history.

