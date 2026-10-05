# fix/claude-login-loop: Test Contract

## Functional Behavior
- A Claude SDK result marked as an error with an authentication failure is forwarded intact, then its stale query is closed and the sidecar exits. The next message can resume the preserved provider session with a new credential reader.
- Ordinary successful turns, quoted login instructions, tool failures, API-key errors, rate limits, and permission failures retain their existing process behavior.
- While the login terminal is still running, a user who completed browser login can choose Check sign-in. A signed-in health result enables Retry message; unknown, absent, failed health reads do not claim success without a vendor exit.
- Vendor exit retains the existing success, failure, cancellation, and retry behavior. Sign-in replies stay confined to the provider terminal.

## Unit Tests
- ProviderSignInDialog: manual confirmation before TerminalExited, unknown health, signed-out health, failure to read health, cancellation, and retry.

## Integration / Functional Tests
- Spawn the real sidecar with a fixture SDK holding its input/query open. Authentication failure must emit its final result before exit and close the query. Non-auth failures and success must accept a second turn.
- Resume a replacement sidecar with the same session id and receive a successful turn.

## Smoke Tests
- bun run build and bun run test must pass on the PR branch.

## E2E Tests
- Automated process and component tests cover the recovery boundaries without replacing vendor credentials or initiating a real OAuth login.
- Real browser OAuth requires user interaction and is recorded as a manual verification limitation.

## Manual / cURL Tests
- In a Claude chat whose session expired, sign in in the browser, choose Check sign-in if the terminal has not ended, and choose Retry message. The conversation must stay intact and the retry must start a fresh Claude process.
- Cancel sign-in and reopen it: the provider link must be available again.
- A normal two-turn Claude chat must continue using one warm query.
