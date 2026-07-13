# Issue 32 Contract: Evidence-backed worker relay

## Guarantee

- Every reported worker result is persisted as the canonical typed record on the parent's active session branch and receives a stable session-entry evidence ID.
- A later sibling delegation references prior active-branch worker evidence by ID by default. The orchestrator may select a subset with `evidenceIds`, but it cannot synthesize or override the stored result.
- Bridge resolves every selected ID against the same parent session and injects the exact validated typed result into the new worker's context packet. Missing, foreign-session, stale-branch, malformed, or non-worker-result IDs fail closed before provider startup.
- The live parent notification is routing metadata (`evidenceId`, status, and summary). SQLite's typed `worker.result` entry remains the record used by later workers.
- Provider projection reduces a parent-visible worker result to routing fields plus `evidenceId`; detailed files, tests, decisions, risks, and remaining work stay in the canonical SQLite record and are hydrated only into selected worker packets.
- Raw worker transcripts are never included.

## Required tests

- Reporting a result returns the parent's durable evidence-entry ID and remains exactly-once.
- Default evidence selection includes prior worker results on the active branch in deterministic order and excludes abandoned-branch results.
- Explicit `evidenceIds` preserve request order, reject duplicates, and cannot resolve another session's entry or a non-result entry.
- Hydration rejects malformed stored payloads instead of falling back to orchestrator prose.
- The worker briefing includes evidence IDs and the exact typed result fields loaded from SQLite.
- Parent delivery and UI audit events expose the evidence ID while retaining the typed result as durable structured data.

## Local refund checkpoint

1. Run the focused evidence-relay and delegation tests.
2. Run the full frontend and Rust suites.
3. Run type/build checks and `git diff --check`.
4. Audit the final diff for transcript leakage, cross-session reads, abandoned-branch reads, nondeterministic ordering, silent fallback, and unbounded context growth.
5. Open one PR for issue #32 only; merge only after every available check passes.
