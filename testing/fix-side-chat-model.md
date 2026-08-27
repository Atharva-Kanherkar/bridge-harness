# fix/side-chat-model — Test Contract

Field report: side chats (**asides**, opened by a `$harness` shortcut from
inside a chat — see `feat-aside-chat.md`) error out, "especially with codex or
opencode", and there is no way to choose which model a side chat begins with —
it silently uses a default the user does not want.

Root cause (read from the code, `origin/main`):

- `openAside` (`src/App.tsx`) creates the side chat with
  `createChat(adapter.id, adapter.defaultModel ?? null, title)`. That baked-in
  default is the only model a side chat can ever begin with: Codex's
  `default_model` is its **Fast**-tier `gpt-5.6-luna`; OpenCode's is whatever
  the catalog reports, and is `null` until the catalog has loaded a provider.
- The aside is the un-fixed sibling of the deferred-new-chat model fix (#350):
  the new-chat draft got an interactive `ChatModelControl`; the aside kept a
  display-only `harness · model` label (`AsideChat.tsx`) and no picker.
- When the resolved model is empty, cold start falls back to the Fast-tier
  model (`live_turn.rs`), and for OpenCode `resolve_model` can fail outright
  when no provider model is selectable, so the side chat starts on the wrong
  model or fails to start.

This change is **frontend-only**. The backend already accepts a chosen model on
`create_chat` and a switch through `update_chat_model`.

## Functional Behavior

- Beginning a side chat resolves a real, selectable model for that harness
  instead of the raw adapter default:
  1. if the side chat's harness matches the chat it was opened from and that
     chat has a model, carry that model (consult the same model you are on);
  2. otherwise use that harness's Standard-tier default model;
  3. otherwise the adapter's `defaultModel`, then its first model;
  4. the model passed to `createChat` is always one the adapter currently
     exposes, or `null` only when the adapter exposes no models at all.
- If the harness exposes no selectable model at all (e.g. OpenCode with no
  connected provider), the side chat is **not created**; a clear error is
  surfaced ("<Harness> has no model available to start a side chat…") instead
  of creating a session that fails at cold start. The existing
  `!adapter.available` guard still fires first for the fully-unavailable case.
- The aside panel header exposes an interactive `ChatModelControl`, wired to
  the aside session, so the model can be picked/switched from the side chat
  itself. Switching applies through `update_chat_model` (same path the main
  chat's control uses) and takes effect on the next message.
- Auto-send of the shortcut's first message is unchanged (locked by
  `feat-aside-chat.md`): the side chat still sends its opening question
  immediately, now on the resolved model.
- No em dashes in any user-visible string. No hedging copy.

## Unit Tests

- `src/asideModel.test.ts` (new, node): the model-resolution helper
  - carries the source session's model when the harness matches
  - falls back to the Standard-tier default when it does not
  - falls back to `defaultModel`, then `models[0]`, then `null`
  - never returns a model id the adapter does not expose
- `src/components/AsideChat.test.tsx` (extended, jsdom):
  - the header renders a model control reflecting the session's harness/model
  - changing the model calls `onChangeModel` with the aside session's harness
    and the chosen model id
  - existing cases (mark/label/title, Escape+scrim close, promote, ⏎ send,
    approval routing) stay green
- `src/App.test.tsx` (extended, jsdom):
  - a `$<harness>` side chat opened from a session that is on a non-default
    model of the same harness begins on that carried model, not the adapter
    default
  - changing the model in the aside panel calls `update_chat_model` for the
    aside session id

## Integration / Smoke

- `bun run check`, `bun run test`, `bun run build` all green.
- Mock mode: `$codex …` from an open chat opens the aside on a Standard model
  (not the Fast default); the header picker switches it; the underlying chat
  never moves.

## E2E

N/A — desktop app.

## Manual Tests (reviewer)

1. Open a chat, switch it to a non-default model of a harness, then
   `$<sameharness> check this` → the aside opens on the same model.
2. `$opencode …` with a connected provider → the aside opens on OpenCode's
   Standard model and answers; with no provider → a clear error, no broken
   session.
3. Change the model in the aside header → the next message runs on it.
