# Issue 40 Contract: Offline policy decision replay

- Every newly persisted policy decision includes a versioned, provider-neutral snapshot of the complete `PolicyInput` needed to reproduce it.
- A read-only offline CLI loads persisted decision entries, re-evaluates replayable inputs against either the current defaults or an explicit candidate configuration, and never starts a provider or mutates the database.
- Reports distinguish exact matches, changed outcomes, transition counts, aggregate route/capability-unit totals, invalid records, and legacy records that predate replay snapshots.
- The current-default replay is a regression gate: the checked-in fixture and newly recorded decisions must reproduce their original decision, reason, and capability-unit charge exactly.
- Candidate reports measure structural routing and authorized capability-unit differences. They do not claim quality, realized cost, or savings without separately collected outcome labels and provider billing data.
- Existing decision payload fields remain stable for the product UI and historical readers; replay metadata is additive and explicitly schema-versioned.

## Local refund checkpoint

Verify exact default replay, deliberate candidate-policy transitions, legacy/invalid accounting, read-only database access, stable existing payload fields, the full frontend/Rust suites, type checks, and `git diff --check` before the issue-only PR.
