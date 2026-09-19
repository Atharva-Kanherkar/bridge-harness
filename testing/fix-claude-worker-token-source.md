# Claude worker credential source — Test Contract

## Functional Behavior
- Interactive Claude sessions are unchanged: Bridge never reads the Keychain for them; the sidecar finds its own sign-in.
- A read-only Claude worker cannot find the user's sign-in because Claude Code scopes credential lookup to the redirected `CLAUDE_CONFIG_DIR`. Bridge hands it a token according to the Claude harness setting `advanced.workerCredentialSource`:
  - `auto` (default): `CLAUDE_CODE_OAUTH_TOKEN` from Bridge's environment, else the `claude` CLI's macOS Keychain entry.
  - `environment`: the environment variable only. The Keychain is never consulted. A missing variable fails the worker launch with a message that names `claude setup-token` and the setting.
  - `none`: nothing is injected.
- The Keychain secret is read at most once per token version: the entry's modification date (readable without an access grant) is the freshness check, so a refreshed token is re-read and an unchanged one never re-prompts.
- When `auto` finds no credential, the worker still launches and its startup diagnostics carry a `worker-credentials` entry saying so, instead of an opaque auth failure later.
- The setting is reachable in Settings → Harnesses → Claude Code as a labelled control; saving it never persists a token, and secret-looking fields in the Claude advanced JSON are rejected.

## Unit Tests
- `claude_adapter`: `mdat` parsing from the real `security` listing; cache reuse on an unchanged date, re-read on a changed date, clearing when the entry is gone or the secret read returns nothing; the source × environment × Keychain resolution matrix, including that `environment` and `none` never call the Keychain.
- `agent_config`: `claude_settings` parses the three values, defaults to `auto`, rejects unknown values and secret fields on save.
- `worker_sandbox`: a sandbox carries the configured source and defaults to `auto`.
- Frontend: `ClaudeHarnessSettings` renders the three options with the stored value and reports a change once, as `{ workerCredentialSource }` only.

## Integration / Functional Tests
- `cargo test -p bridge-core claude_adapter:: agent_config:: worker_sandbox::` green.
- `bunx vitest run src/components/ClaudeHarnessSettings.test.tsx src/components/settings` green; `tsc -b` clean.

## Manual Tests
- With the entry present: launch two read-only Claude workers; `security` is consulted for the secret once, and the second launch reuses the token.
- Run `claude login` (recreates the entry) then launch a worker: the secret is re-read exactly once.
- Set the source to `environment` without the variable: the launch fails with the actionable message. Set it with `claude setup-token` exported: the worker authenticates without any Keychain access.
