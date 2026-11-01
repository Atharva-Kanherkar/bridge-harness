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
- A Claude account launch removes inherited credentials that outrank the
  folder's login (`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`,
  `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK|VERTEX|FOUNDRY`) and an
  inherited endpoint or headers (`ANTHROPIC_BASE_URL`,
  `ANTHROPIC_CUSTOM_HEADERS`), so neither another credential nor the
  account's own token reaches an inherited endpoint. Generic `HTTPS_PROXY`
  stays (it tunnels TLS). The credential policy judges that environment, so
  an API-key-only deployment refuses an account. Codex reads its login from
  `CODEX_HOME` alone.
- Shadow and shared homes are compared by where they resolve, before either
  is touched: a shadow that is, sits inside, or contains the shared home
  (directly or through a symlink) is refused and the shared home is left
  intact.
- Shadow-home setup is serialised per shadow folder, and a link another
  launch has just created with the right target is accepted, so two chats
  starting on one new account both launch.
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
- The aside picker shows and sends the account exactly like the main
  composer.
- A specialist agent target runs on the user's own login, so choosing one
  clears the account in the draft and resolved settings never carry one.
- A fork or an aside on the same agent keeps the source chat's account (the
  only home its forked thread can be read from); one on another agent starts
  on the user's own login.
- An unstarted aside holds its source's thread id only to fork it. A switch
  that keeps the thread keeps that fork instruction; one that cannot reach
  the thread clears the id. Neither ever resumes the source.

### UI
- Settings → Coding agents → Claude Code / Codex has an **Accounts** group:
  lists added accounts, removes one, and adds one with a name and a folder.
  Codex has a "Share chats with your main Codex" switch (shadow mode, on by
  default). The add form shows the exact sign-in command for the folder, e.g.
  `CLAUDE_CONFIG_DIR=~/.claude_personal claude auth login`, with the folder
  shell-quoted (keeping a leading `~/`) when it holds spaces or metacharacters.
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
- `provider_accounts::tests::explicit_claude_account_drops_inherited_credentials`
- `provider_accounts::tests::the_credential_policy_judges_the_account_launch_environment`
- `provider_accounts::tests::concurrent_first_launches_share_one_shadow_home`
- `provider_accounts::tests::a_link_created_by_another_launch_is_accepted`
- `provider_accounts::tests::an_account_launch_never_reaches_an_inherited_endpoint` (real `claude`, synthetic credentials, local recorders; skipped when `claude` is not installed)
- `provider_accounts::tests::a_shadow_that_aliases_the_shared_home_is_refused_untouched`
- `sessions::tests::a_pending_aside_still_forks_its_source_after_a_same_home_switch`
- `agent_config::tests::save_harness_validates_accounts`
- `sessions::tests::account_only_switch_is_a_change`
- `sessions::tests::account_switch_across_homes_clears_the_thread`
- `sessions::tests::account_switch_inside_a_codex_shadow_group_resumes_natively`
- `sessions::tests::absent_account_keeps_the_chat_account_on_same_harness`
- `sessions::tests::unknown_account_is_rejected`
- `sessions::tests::asides_on_the_same_agent_keep_the_account`
- `chat_defaults::tests::configured_chat_stores_the_chosen_account`
- `chat_defaults::tests::unknown_account_in_choice_is_rejected`
- `chat_defaults::tests::a_specialist_target_drops_the_account`
- `store` migration test: schema 68 adds `sessions.account`.
- `live_turn`: launch resolves the account home and refuses a removed account.

Frontend (Vitest):
- `providerAccounts.test.ts` — parse accounts from harness config, login command text, group labels.
- `ChatModelControl.test.tsx` — account groups render; picking emits `(harness, model, account)`; selected row matches account; no accounts = unchanged.
- `ProviderAccountsSettings.test.tsx` — add (writes `advanced.accounts` and keeps other keys), remove, validation, codex share switch → `shadowHomePath`, login command shown.
- `providerAccounts.test.ts` — sign-in command quotes folders with spaces and shell metacharacters.
- `AsideChat.test.tsx` — the aside picker shows the account and emits the account picked.
- `App.deferNewChat.test.tsx` — picking a specialist after an account clears it.

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

## Extension: one driver table, OpenCode and Cursor

T3 Code supports several instances of every driver (`supportsMultipleInstances`
defaults to true), each with its own environment and driver config, and a
default continuation key of `<driver>:instance:<id>`. Bridge ports that as one
account-driver table, so a new agent is one entry plus one call in its adapter.

| Agent | How an account keeps its own login | Sign-in | Threads continue |
| --- | --- | --- | --- |
| Claude Code | `CLAUDE_CONFIG_DIR` folder | `claude auth login` in that folder | within one folder |
| Codex | `CODEX_HOME` folder or shadow home | `codex login` in that folder | within one shared home |
| OpenCode (Go) | `XDG_DATA_HOME` folder whose `opencode/` is private and every other entry links to the real data home, so commands the agent runs see the user's usual data | in app: Bridge hands the key to a private OpenCode server for that folder; OpenCode stores it, Bridge does not | within one folder |
| Cursor | its own API key as `CURSOR_API_KEY` (Cursor keeps the CLI login in one global Keychain item, so a folder cannot isolate it) | in app: the key goes to the macOS Keychain under Bridge's own service, never into settings | only within one account |

### Functional behavior
- `advanced.accounts` is accepted on `claude`, `codex`, `opencode` and `cursor`;
  any other harness is rejected.
- OpenCode accounts need a folder and no shadow; Cursor accounts take no folder.
- An OpenCode account launch sets `XDG_DATA_HOME` to the overlay folder and
  removes inherited `OPENCODE_API_KEY`. The overlay keeps `opencode/` real and
  links every other entry of the user's data home; it is refused when it
  resolves to, inside, or around the user's data home.
- A Cursor account launch sets `CURSOR_API_KEY` from the Keychain and blanks
  inherited `CURSOR_AUTH_TOKEN`. A Cursor account with no saved key fails to
  start with a message that says where to add it.
- `config/set_account_api_key {harness, account, apiKey}` saves a Cursor key;
  it refuses folder-based agents and unknown accounts. Removing an account
  deletes its saved key.
- `config/set_opencode_provider_api_key` takes an optional `account` and then
  connects the key inside that account's folder.
- The launch reports secrets by variable name only; no key appears in Debug
  output, errors, or the settings record.
- Claude and Codex behave exactly as before.

### UI
- Settings → Coding agents → OpenCode and Cursor get the same Accounts group.
  OpenCode asks for a name, a folder and an optional OpenCode Go key; Cursor
  asks for a name and an API key. Each saved account can replace its key.
- The model picker, aside and new-chat draft offer OpenCode and Cursor
  accounts like the others.

### Tests
- `provider_accounts::tests::the_driver_table_names_every_supported_agent`
- `provider_accounts::tests::opencode_and_cursor_accounts_validate_by_login_kind`
- `provider_accounts::tests::an_opencode_overlay_keeps_its_data_private_and_links_the_rest`
- `provider_accounts::tests::an_opencode_overlay_that_aliases_the_data_home_is_refused`
- `provider_accounts::tests::a_cursor_account_launch_hands_over_its_key_and_hides_it`
- `provider_accounts::tests::a_cursor_account_without_a_key_names_where_to_add_one`
- `provider_accounts::tests::continuation_keys_follow_the_login_kind`
- `account_secrets::tests::saved_keys_round_trip_and_removal_deletes_them`
- `providerAccounts.test.ts` — OpenCode and Cursor choices, labels, sign-in hints.
- `ProviderAccountsSettings.test.tsx` — OpenCode account with a Go key; Cursor account with an API key; replacing a key.
