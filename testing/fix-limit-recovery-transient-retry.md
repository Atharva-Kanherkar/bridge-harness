# fix/limit-recovery-transient-retry: Test Contract

Locked before implementation.

## Why

When a chat hits a usage limit, limit recovery waits for the reset, then continues the chat (or switches it to the backup model). If the store is briefly busy at that moment, the action fails with `Database: database is locked`. The row then goes straight to `failed`, and the chat is never continued. Nothing retries it.

CI has shown this more than once. One `j5_a_pending_recovery_survives_a_restart_and_fires_after_it` run ended with exactly that outcome. Other runs of the same journeys timed out with too little output to say why. Those failures were in unrelated PRs, and on `main` (j1).

## Decisions

| # | Decision | Why |
| --- | --- | --- |
| D1 | A recovery action that fails because SQLite was busy or locked goes back to `pending` with the outcome `Retrying after: <error>`. Any other failure still ends `failed`. | A busy store says nothing about the chat. |
| D2 | A transient retry happens only within 2 minutes of the row coming due. After that, the row fails with the error. | A lock that lasts minutes is not transient, and retrying for 12 hours would be noise. |
| D3 | The next attempt goes through the guard again before acting. If the failed attempt had started a turn, the chat is "already running" and nothing is sent twice. | No duplicate continue. |
| D4 | The journeys' wait helpers print the recovery row and the chat's status on timeout. | The next flake says why it failed. |
| D5 | The journeys create their chat with `sessions/create_chat_id`, so they act on the id they made, not the first new id in a session list. | It removes a race on another session appearing. This is not proven to cause any failure. |

## Tests

- Unit: a send that fails with SQLite busy is retried on the next tick, and the chat is continued exactly once.
- Unit: a switch that fails with SQLite busy is retried, and the row is done.
- Unit: a busy failure outside the 2-minute window fails the row.
- Unit: a non-transient failure still fails at once (existing test).
- Unit: if a turn started before the retry, the retry is cancelled as "already running" and nothing more is sent.
- The existing fuzz (`store_and_tick_interleavings`) holds, with busy errors added to the failures it can inject.
- Journeys j1 to j5 pass. A timeout prints the row and the chat's status.

## Manual / cURL

N/A.
