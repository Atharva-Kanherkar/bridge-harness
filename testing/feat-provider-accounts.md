# feat/provider-accounts — Test Contract

Use more than one Claude Code or Codex subscription in Bridge, side by side, the
way T3 Code does it. Ported from T3 Code's provider instances
(`pingdotgg/t3code` at `97e35bcf`, MIT): `packages/contracts/src/providerInstance.ts`,
`packages/provider-core/src/server/{instanceEnvironment,pathExpansion}.ts`,
`apps/server/src/provider/Drivers/{ClaudeHome,CodexHomeLayout}.ts`, and the
docs in `docs/user/providers-{claude,codex}.md`.

The T3 model, and how it maps onto Bridge:

| T3 Code | Bridge |
| --- | --- |
| Provider instance `{driver, displayName, config.homePath, config.shadowHomePath}` | Account `{id, label, homePath, shadowHomePath}` in the `claude` or `codex` harness's `advanced.accounts` |
| Default instance id = driver kind, inherits the environment | No account (`null`) or the harness id = the user's own login |
| `ModelSelection {instanceId, model}` | `ChatModelChoice {harness, model, account}` and `sessions.account` |
| `CLAUDE_CONFIG_DIR` set from `homePath`, never `HOME` | Same |
| `CODEX_HOME` from `homePath`, or a shadow home that symlinks shared state and keeps `auth.json` private | Same |
| Continuation group key `claude:home:<dir>` / `codex:home:<shared dir>` | A switch resumes the provider thread only inside one key; otherwise it hands over like a harness switch |

## Functional Behavior

### Account configuration
- `advanced.accounts` on the `claude` and `codex` harness is an array of
  `{id, label, homePath, shadowHomePath?}`. Other `advanced` keys are untouched.
- `id` matches `^[a-zA-Z][a-zA-Z0-9_-]*$`, is at most 64 characters, is unique
  within the harness, and is not the harness id itself (that id names the
  user's own login, as T3's `defaultInstanceIdForDriver` does).
- `label` is trimmed, non-empty, at most 64 characters.
- Claude: `homePath` is required and becomes `CLAUDE_CONFIG_DIR`.
  `shadowHomePath` is rejected.
- Codex: `homePath` (empty means the inherited `CODEX_HOME`, else `~/.codex`)
  and optional `shadowHomePath`. At least one is non-empty. A shadow equal to
  the shared home is rejected.
- Accounts on any other harness are rejected by `config/save_harness`.
- `~`, `~/x`, and `~\x` expand against the home directory; anything else is
  used as written (T3 `expandHomePath`).

### Launch
- A chat with no account launches exactly as before: no `CLAUDE_CONFIG_DIR`
  or `CODEX_HOME` is added.
- A Claude chat on account `personal` with `homePath = ~/.claude_personal`
  launches the sidecar with `CLAUDE_CONFIG_DIR=$HOME/.claude_personal`;
  `HOME` is not changed (Keychain lookup stays intact).
- A Codex chat on a direct account launches `codex app-server` with
  `CODEX_HOME=<homePath>`.
- A Codex chat on a shadow account first materialises the shadow home:
  every shared entry except `auth.json`, `auth.lock`, `models_cache.json`,
  `log`, `memories`, `tmp`, `.DS_Store` is a symlink to the shared home; a
  private entry that is a symlink is removed (or refused for `auth.json`); a
  real file blocking a shared link is an error naming the entry; the runtime
  lock dirs are replaced. Then `CODEX_HOME=<shadow>`.
- Read-only worker sandboxes keep their own isolated homes; account env never
  overrides the sandbox.
- A chat bound to an account that was since removed fails to start with a
  message naming the account and telling the user to choose another one.

### Selection and switching
- `ChatModelChoice` carries an optional `account`. Chat defaults, draft
  overrides, and `chats/create_configured` validate it against the harness's
  accounts and store it on the new session row.
- `sessions/update_chat_model` takes an optional `account`:
  - absent: keep the chat's account when the harness is unchanged, otherwise
    the user's own login;
  - the harness id: the user's own login;
  - an account id: that account (must exist).
- A change of account alone is a real change (not a no-op).
- The stored provider thread is resumed natively only when the harness is
  unchanged, the adapter resumes natively, a thread exists, and the old and new
  accounts share a continuation key. Otherwise the provider session id is
  cleared and the next turn starts fresh with the projected history.
- `Session.account` is reported in state so every client shows it.

### UI
- Settings → Coding agents → Claude Code / Codex has an **Accounts** group:
  lists added accounts, removes one, and adds one with a name and a folder.
  Codex has a "Share chats with your main Codex" switch (shadow mode, on by
  default). The add form shows the exact sign-in command for the folder, e.g.
  `CLAUDE_CONFIG_DIR=~/.claude_personal claude auth login`.
- The model picker shows one group per account ("Claude Code · Personal")
  beside the default group; picking a model there switches the chat to that
  account. The pill label names the account.
- The new-chat draft picker does the same, and the created chat starts on the
  chosen account.
- With no accounts configured the picker and settings look exactly as before.

## Unit Tests

Rust (`bridge-core`):
- `provider_accounts::tests::expands_home_relative_paths` — `~`, `~/a`, `~\a`, `/abs`, `rel`, empty.
- `provider_accounts::tests::parses_valid_accounts_and_ignores_other_advanced_keys`
- `provider_accounts::tests::rejects_bad_ids_duplicates_and_reserved_harness_id`
- `provider_accounts::tests::rejects_shadow_home_on_claude_and_empty_claude_home`
- `provider_accounts::tests::rejects_accounts_on_unsupported_harness`
- `provider_accounts::tests::claude_home_and_continuation_key`
- `provider_accounts::tests::codex_direct_and_shadow_layouts_share_continuation_keys`
- `provider_accounts::tests::materializes_shadow_home_with_shared_links_and_private_auth` (ports T3 test)
- `provider_accounts::tests::shadow_home_keeps_local_entries_local` (ports T3 Finder test)
- `provider_accounts::tests::shadow_home_refuses_real_file_blocking_a_shared_link`
- `provider_accounts::tests::shadow_home_refuses_symlinked_auth`
- `provider_accounts::tests::shadow_home_equal_to_shared_home_is_refused`
- `agent_config::tests::save_harness_validates_accounts`
- `sessions::tests::account_only_switch_is_a_change`
- `sessions::tests::account_switch_across_homes_clears_the_thread`
- `sessions::tests::account_switch_inside_a_codex_shadow_group_resumes_natively`
- `sessions::tests::absent_account_keeps_the_chat_account_on_same_harness`
- `sessions::tests::unknown_account_is_rejected`
- `chat_defaults::tests::configured_chat_stores_the_chosen_account`
- `chat_defaults::tests::unknown_account_in_choice_is_rejected`
- `store` migration test: schema 68 adds `sessions.account`.
- `live_turn`: launch resolves the account home and refuses a removed account.

Frontend (Vitest):
- `providerAccounts.test.ts` — parse accounts from harness config, login command text, group labels.
- `ChatModelControl.test.tsx` — account groups render; picking emits `(harness, model, account)`; selected row matches account; no accounts = unchanged.
- `ProviderAccountsSettings.test.tsx` — add (writes `advanced.accounts` and keeps other keys), remove, validation, codex share switch → `shadowHomePath`, login command shown.

## Integration / Functional Tests
- Protocol artifacts regenerated; `cargo test -p bridge-protocol` (schema drift, params strictness, mirror) green.
- `protocol_mirror::result_payloads_mirror_core` covers `Session.account`.
- bridge-deck command signature test covers the new `account` argument.
- `bun run test` and `bun run build` green.

## Smoke Tests
- `bun run check` green.
- Mock-mode app (`bun run dev`): Settings → Claude Code → Accounts → add "Personal" → picker shows "Claude Code · Personal".

## E2E Tests
- Manual on a real install (needs two subscriptions):
  1. `mkdir -p ~/.claude_personal && CLAUDE_CONFIG_DIR=~/.claude_personal claude auth login`
  2. Settings → Coding agents → Claude Code → Accounts → add "Personal", folder `~/.claude_personal`.
  3. New chat → picker → Claude Code · Personal → send. `ps eww` on the sidecar shows `CLAUDE_CONFIG_DIR=…/.claude_personal`; the reply arrives billed to the second account.
  4. Switch the same chat to plain Claude Code → next turn starts fresh with history projected.

## Manual / cURL Tests
N/A — no HTTP surface. The daemon socket accepts `sessions/update_chat_model`
with `{"sessionId":…,"harness":"claude","account":"personal"}`.

## Out of scope (documented in the PR)
- In-app sign-in per account (T3's managed Codex auth flow). Users sign in
  with the CLI command the settings page shows.
- Per-account sign-in status, model catalogues, usage meters, and limits.
- Workers, titles, suggestions, and other background jobs run on the user's
  own login.
