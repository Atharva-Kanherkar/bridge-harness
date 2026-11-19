# feat/import-continue: Test Contract

Issue #1061. A person can keep talking to a chat imported from Claude Code. When Claude Code still has the session and Bridge can sign in where it lives, the next reply continues that same Claude Code session. Otherwise Bridge starts a new chat seeded with the imported history. The composer says which will happen before the first send, and the imported chat itself never changes.

## Functional Behavior

- An imported chat stays immutable. Continuing it creates a new ordinary chat that branches from the imported chat's head: a copy of its history with new entry ids, recorded as a fork of the imported chat. The imported chat's entries are byte-identical before and after.
- The new chat continues natively when all of these hold:
  - Bridge recorded the Claude Code session id at import.
  - That session's transcript still exists in the Claude config folder of an account Bridge signs in with: the user's own login, or a Claude account in Settings. The new chat signs in with that account.
  - The chat's folder still exists.
  - No other Bridge chat holds that Claude Code session.
- Native continuation sets the Claude Code session id on the new chat, so its first start resumes that session, and the next reply continues the same session id. If the resume fails at launch, the existing fallback restores from the copied history instead, and says so.
- Otherwise the new chat starts fresh, seeded with the imported history through the existing checkpoint path, and the reason is given:
  - Claude Code no longer has this session.
  - This chat's folder no longer exists.
  - The session is saved under a Claude Code folder Bridge does not sign in with.
  - Another Bridge chat already continues this session.
  - Bridge did not record which Claude Code session this was.
- The composer on an imported chat shows, before any send, either "Continues the Claude Code session" or "Starts a new chat with this history" with the reason. It also says that unfinished tool calls, background processes and hidden reasoning stay behind.
- Sending from an imported chat creates the continuation, opens it, and delivers the message there. The imported chat keeps its "Imported" badge; the continuation shows in the sidebar as forked from it, through the existing fork relation.
- The continuation gives the copied history to the model. Imported entries are stored for display only, so the copies a continuation or a fork makes of them are marked for context.
- Forking an imported chat (sidebar Fork) makes an ordinary chat that can start. Today the fork copies `kind = imported` and cannot start.
- Restarting Bridge keeps the continuation's history, its Claude Code session id and its restoration mode.

## Unit Tests

- Eligibility matrix (`import_continue`): native when everything holds; history with the right reason for each of a missing transcript, a missing folder, a transcript under a folder no account uses, a session another chat holds, and no recorded session id.
- Account resolution: a transcript under a configured Claude account's folder picks that account.
- `fork_session_records` turns an imported parent into a `direct` child, and its copied entries count for context.
- A history continuation's restoration context holds the imported conversation; the imported chat's own does not.

## Integration / Functional Tests

- Continuing natively on a real store creates a `direct` chat with the copied history, `provider_session_id` set to the Claude session id, and a head in `native` restoration mode. The imported chat's entries are unchanged, byte for byte.
- Continuing with history mode creates a `direct` chat with the copied history, no provider session id, and a `checkpoint_restored` head.
- `live_turn::start_chat` accepts the continuation and refuses the imported chat as before.
- The restoration plan for a native continuation is `Native`, and for a history continuation `CheckpointRestored`.
- Protocol and shell contract tests cover the two new methods.
- `MoveToBridge` is unaffected. In the chat view, an imported chat shows the continuation notice, and sending calls continue, then sends to the new chat.

## Smoke Tests

- `bun run build`
- `cargo test -p bridge-core --lib import_continue`, `cargo test -p bridge-protocol`, `cargo test -p bridge-deck --lib`

## E2E Tests

- Real Claude Code: make a real session with `claude -p` in a scratch folder, import it, and continue it from Bridge. The reply continues the same Claude Code session id, and the transcript gains the new turn. Then delete the transcript, continue a second import of it, and see the new chat start with the history and the reason.

## Manual Tests

- Open an imported chat: the notice above the composer says which way it will continue.
- Send: Bridge opens the new chat with the history and the reply. The imported chat is unchanged and links to it.
