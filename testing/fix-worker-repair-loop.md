# Worker repair loop contract (issue #572, slice 1)

- A provider failure is not a formatting error: no result-repair turn is sent.
- Only a working worker with a pending result may spend its one repair attempt.
- The attempt is claimed in SQLite before sending, survives restarts, and is not refunded on delivery failure or terminal parsing. Existing repair events seed the budget during migration.
- Failed and protocol-invalid results settle to completed and stop the adapter. Duplicate terminal frames cannot send turns or report another result.
- Maintenance reaps already-reported failed workers immediately and unreported failed workers after the stall timeout, without interrupting a normal retry transition.

Verification: replay Codex usage-limit frames repeatedly through the live handler; test repair exhaustion with a fresh in-memory tracker, non-working/reported gates, failed delivery, migration backfill, adapter cleanup and maintenance. Run `bun run build` and `bun run test` before opening the PR.
