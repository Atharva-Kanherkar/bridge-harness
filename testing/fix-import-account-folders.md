# fix/import-account-folders: Test Contract

Left over from #1053: the one-button import reads `~/.claude` and `CLAUDE_CONFIG_DIR`, but not the Claude accounts added in Settings, so their chats never come over. Codex and OpenCode have the same gap: an account with its own Codex home, or its own OpenCode data folder, is not read either.

Where each tool keeps an account's chats, from `provider_accounts.rs`:

- Claude Code: the account's config folder (`homePath`, its `CLAUDE_CONFIG_DIR`).
- Codex: the account's shared Codex home (`homePath`). An account with only a `shadowHomePath` is a login overlay on your own home, so its chats are already in it.
- OpenCode: the account's data home (`homePath`, its `XDG_DATA_HOME`), whose `opencode/` folder holds the account's own store.

## Functional Behavior

- Importing from a tool reads your own folder and every account folder added for that tool in Settings. A folder that does not exist is skipped. A folder reached twice (an account pointing at your own folder) is read once.
- The same session in two folders comes over once, as before.
- An import Bridge quit in the middle of resumes over the same folders, account folders included.
- The Import page counts what every folder holds, account folders included, and says when a tool's chats come from more than one folder ("from 2 folders").
- Continuing an imported chat resumes natively in whichever of your folders still holds the session, and signs in with that folder's account. This already held for Claude Code and OpenCode accounts; it now holds for Codex accounts too.
- Nothing changes for someone with no accounts.

## Unit Tests

- The sign-in folders for each tool: your own, then each account's, by the rules above.
  - Claude: `homePath`.
  - Codex: `homePath`. A shadow-only account is your own home.
  - OpenCode: `homePath`'s `opencode/`.
- The job's folders for each tool: your own plus accounts, missing ones skipped, duplicates once.
- Detection counts across your folder and account folders for Claude Code, Codex and OpenCode.
- `sourceSummary`: "from 2 folders" when a tool has more than one folder; nothing extra for one.

## Integration / Functional Tests

- With a Claude account folder in Settings holding a chat that `~/.claude` does not, the job imports both, and a second run imports nothing new.
- The same for a Codex account home and an OpenCode account data folder.
- A Codex account's imported chat plans a native continuation signed in with that account.

## Smoke Tests

- `cargo test -p bridge-core --lib -- import_job import_continue import_sources`
- `npx vitest run src/moveToBridge.test.ts src/components/MoveToBridge.test.tsx`
- `bun run build`

## E2E Tests

- Scripted fake user, with a Claude account folder added in Settings on the fake home: the Claude section counts its chats and says it reads from 2 folders, and after the import the account's chat is in the sidebar.

## Manual Tests

- On a Mac with a second Claude account in Settings: Import, then Claude Code, brings over that account's chats too.
