# Issue 10 — Context Projection and Compaction Test Contract

This contract locks the definition of done for GitHub issue #10. It inherits the umbrella contract in `testing/issue-1-subscription-native-session-forest.md` and does not expand beyond context projection, checkpoints, and controller-owned compaction.

## Functional Behavior

- `ContextProjector` deterministically walks the active session-forest branch, applies only the newest compaction boundary, and returns render entries, compact restoration context, current model/effort, token estimate, and context pressure.
- Repeated compactions preserve immutable raw entries and durable decisions; a compaction records schema version, summary, first retained entry ID, tokens before, files touched, reason, and source agent.
- Checkpoints use a strict versioned schema. Every resumable agent requests its own checkpoint through its existing harness session with the exact checkpoint-only instruction; invalid output receives one same-session repair turn and then fails explicitly.
- `CompactionController`, not the model, decides when to compact: context at least 75%, projected response reserve, phase boundary, before suspend/downgrade/shutdown, or manual request.
- Compaction is skipped for one-shot workers that already have a valid typed result, during a tool call or approval, when no meaningful work was added, and for wall-clock age alone.
- Failure appends immutable `compaction.failed`, retries once, then permits a recovery-worker reconstruction from normalized forest events and Git facts labeled `reconstructed`; shutdown/suspension cannot block forever.
- Orchestrator projections contain typed worker results and never raw child-worker logs.

## Unit Tests

- Deterministic projection: identical forest state produces identical output; newest boundaries win across at least three compactions while decisions remain present.
- Checkpoint schema accepts every required field, rejects unsupported/malformed payloads, and preserves source-agent ownership.
- Trigger matrix covers every `CompactionReason` and every do-not-compact rule, including one-shot typed results and mid-tool/approval state.
- Split and large turns produce stable token estimates and retain the first post-boundary entry.
- Invalid checkpoint output requests exactly one repair; a second invalid result records failure without deleting raw events.
- Orchestrator projection selects `worker.result` summaries and excludes raw worker event streams.

## Integration / Functional Tests

- Three successive compactions on one session retain raw history and durable decisions while restoration uses only the newest valid compact boundary plus retained entries.
- Crash/failure during checkpoint generation leaves the pre-existing active branch intact and queryable.
- Before-suspend lifecycle integration requests/records compaction without introducing an unbounded wait.
- Recovery reconstruction is explicitly labeled and auditable.

## Verification

- `bun run test`
- `bun run check`
- `bun run build`
- `git diff --check origin/main...HEAD`

## Manual / E2E Notes

- No HTTP/cURL surface exists; Bridge is a local Tauri application.
- The deterministic forest/controller integration tests are the automated E2E-equivalent for this backend phase. Live provider checkpoint turns remain runnable only where an authenticated local Codex or Claude harness is installed.
