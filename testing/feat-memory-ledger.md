# feat/memory-ledger

Thin explicit ledger. Not session recall. Not the learning router. Not a Memory UI.

## Schema 29

- Create `memory_records` (named `scope_key TEXT NOT NULL`, never NULL).
- `DROP TABLE IF EXISTS task_knowledge` with **no** `INSERT SELECT`. It never had a production reader.

## Save / list / forget

- Save always writes `account:local`, provenance `user_explicit`. No extract method.
- List requires `scopeKey`; empty/whitespace rejected; list does not return other scopes.
- Forget tombstones (`status=deleted`); list omits tombstones.
- Secret-shaped bodies refused (same interceptors as turns).
- `/pin`, `/pins`, `/unpin` are Bridge-local (`slash::is_bridge_local`); they do not auto-switch harness.

## Out of scope

Time decay, fingerprint clustering, catalog-snapshot dedupe, evaluator executor, Hermes sidecar, injecting pins into prompts, FTS on the ledger, migrating `task_knowledge` rows, Memory review-queue UI.
