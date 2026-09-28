# feat/memory-ledger

This describes the initial thin explicit-ledger slice. Later slices add the
Memory UI, reviewed and automatic extraction, and bounded future-session
packets; see `feat-memory-extract.md` and `feat-memory-packet.md`. It remains
separate from session recall and the learning router.

## Schema 31

- Create `memory_records` (named `scope_key TEXT NOT NULL`, never NULL).
- `DROP TABLE IF EXISTS task_knowledge` with **no** `INSERT SELECT`. It never had a production reader.

## Save / list / forget

- In this slice, save always writes `account:local`, provenance `user_explicit`.
- List requires `scopeKey`; empty/whitespace rejected; list does not return other scopes.
- Forget tombstones (`status=deleted`); list omits tombstones.
- Secret-shaped bodies refused (same interceptors as turns).
- `/pin`, `/pins`, `/unpin` are Bridge-local (`slash::is_bridge_local`); they do not auto-switch harness.

## Out of scope for this initial slice

Time decay, fingerprint clustering, catalog-snapshot dedupe, evaluator executor, Hermes sidecar, injecting pins into prompts, FTS on the ledger, migrating `task_knowledge` rows, Memory review-queue UI.
