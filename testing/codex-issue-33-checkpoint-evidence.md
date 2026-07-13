# Issue 33 Contract: Independent checkpoint evidence verification

## Guarantee

- Before an agent-authored checkpoint commits, Bridge derives required decisions and touched files from immutable active-branch entries since the previous compaction boundary.
- Required decisions come from typed worker-result and checkpoint decision arrays. Required files come from typed worker-result `filesChanged` arrays and `artifact.created` paths.
- The checkpoint must contain every derived item exactly after whitespace normalization. Schema-valid but incomplete checkpoints enter the existing one-repair path; a second incomplete response records `compaction.failed` and does not move the boundary.
- Verification runs in Rust against SQLite history, not in the context-pressured model. Successful verification writes an auditable event with deterministic evidence counts.
- Reconstructed checkpoints remain explicitly marked `reconstructed`; their source is the independent durable-history recovery path.

## Required tests

- A schema-valid checkpoint omitting a durable decision requests repair, then fails if still incomplete.
- A schema-valid checkpoint omitting a touched file requests repair.
- Evidence before the newest completed compaction is not required again.
- Duplicate history evidence is deduplicated deterministically.
- A complete checkpoint commits and records the verifier audit event.
- The full compaction, projection, frontend, and Rust suites remain green.

## Local refund checkpoint

Run focused semantic-verifier tests, the full suites, build/type checks, and `git diff --check`; audit failure atomicity and active-branch/boundary scoping before opening the issue-only PR.
