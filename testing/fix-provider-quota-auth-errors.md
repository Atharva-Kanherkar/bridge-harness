# fix/provider-quota-auth-errors — Test Contract

A user hit a Codex usage limit, logged out in a terminal and signed back in with
an API key, then switched the chat to OpenCode. Bridge told them, in order: that
their Codex plan was exhausted, that they should run `/login`, that their
*OpenCode* plan was exhausted — and it said each of those twice. Three of those
four statements were things Bridge did not know.

The error prose is built in one place, `src/errors.ts`, from a single regex
sweep over the provider's own words. Every match on the usage family produced
"You're out of usage on your <provider> plan's current limit", every match on
the auth family produced "Run its /login command", and the provider name came
from whichever harness the session happened to be set to *now* rather than from
the frame that failed. The doubling is separate and lives in the projection
merge: an error frame carries no provider item id, so its live row and its
durable twin get two different identities and both render.

## Functional Behavior

### Rate limit is not an exhausted subscription

- Classification splits the old `usage-limit` kind in two. `usage-limit` means
  the provider said the account is spent ("usage limit", "quota", "out of
  credits", "insufficient_quota", "monthly/weekly/daily limit"). `rate-limit`
  means the provider said it is throttling ("rate limit", "429", "too many
  requests", "retry-after") without saying anything about exhaustion.
- A `rate-limit` never claims the plan or subscription is used up. Its copy
  names the throttle, says explicitly that it is not proof the plan is spent,
  and offers waiting or switching.
- When a usage snapshot is on hand and its most-pressed window is below the
  cap, a `rate-limit` cites that headroom as the evidence.
- Both kinds keep the warning tone (a wait, not a failure) that `usage-limit`
  had, so the cards and the toast do not regress to the destructive treatment.

### Authentication keeps the API-key/sign-in distinction

- `describeError` reports an `authMode`: `api-key` when the text blames a key
  ("invalid api key", `OPENAI_API_KEY`), `subscription` when it blames a
  session ("not logged in", "please sign in", "session expired"), `unknown`
  otherwise (a bare `401`, `403`, "unauthorized").
- `/login` is prescribed only for `subscription`. An `api-key` failure points
  at the key and says that signing in is a different credential. An `unknown`
  failure states both branches without choosing.

### An error keeps the runtime it came from

- The transcript envelope carries the originating adapter, read from
  `providerMeta.adapter` live and from `payload.providerMeta.adapter` on a
  durable entry, and the reducer stamps it on the item as `harness`.
- An error card names the harness of its own frame. Switching the chat from
  Codex to OpenCode does not relabel the Codex errors already in the
  transcript, and does not attach the new harness's usage snapshot to them.
- When the text names an upstream vendor that is not the runtime's own
  (an OpenAI 429 surfaced through OpenCode), the copy attributes the limit to
  the upstream account, not to an OpenCode plan.

### One failure renders once

- `mergeConversationProjections` treats an error row read from the forest and
  the live row it is a twin of as one row, matched on the stable forest entry
  ID stamped by Bridge on both live and replayed errors, and drops the live copy in
  favour of the durable one.
- Both doors read an error's text from the same places, so a failure whose
  words live in `data.error.message` is not two different rows saying two
  different things.
- Two genuinely distinct failures are still two rows, including two failures
  with identical text at different points in the transcript and two failures
  that arrive only live, even after an earlier identical error leaves the live window.

## Unit Tests

- `src/errors.test.ts`: the classifier separates throttles from exhaustion
  across the phrasings Codex, Claude and OpenCode actually emit; a 429 never
  produces exhaustion copy; the three `authMode`s produce three different
  instructions and only one of them mentions `/login`; an upstream vendor named
  inside another runtime's error is attributed upstream.
- `src/conversation.test.ts`: an error present in both projections renders
  once; two distinct errors render twice; a live-only error survives.
- `src/transcript/codec.test.ts`: both normalizers read the adapter off
  `providerMeta` onto the envelope.

## Not Fixed Here

`src/providerLogin.ts` no longer offers subscription sign-in for a rejected
API key, so the flow that would have undone the user's key swap is closed. The
deeper defect behind "I signed in and it still says sign in" is not:

- A provider child is spawned once per session and reused across turns. The hot
  path returns early when harness, model and prompt prefix are unchanged
  (`live_turn.rs:1670-1703`, `:1115-1141`), and there is no auth-aware teardown
  anywhere in `live_turn.rs`, no filesystem watcher on `auth.json`, and no
  re-handshake on credential change. A prompt retry after `codex logout` /
  `codex login --api-key` writes down the same stdin pipe to the process that
  read the old credentials. The only recoveries are the 120s idle reaper
  (`runtime_budget.rs:14-15`), switching model or harness, or restarting Bridge.
- `AuthState` is a presence-and-non-zero-length stat of `auth.json`
  (`codex_adapter.rs:781-788`), so a subscription-to-API-key swap leaves it
  reading `SignedIn` both before and after — it cannot notice a change of
  credential, only of existence.
- The frontend holds `health` in a query with `staleTime: 30_000`,
  `refetchOnWindowFocus: false` and no interval, invalidated only by an in-app
  login, an `AdaptersChanged` event, or the model-catalog refresh button
  (`src/serverState.ts:32`, `src/App.tsx:321-336`, `:2601`, `:2757`). An
  out-of-app credential change fires none of them.

Fixing that means an auth-aware runtime teardown and an invalidation trigger
for out-of-app changes. It is a backend lifecycle change that wants a live
provider to verify, and is deliberately not attempted blind here.

## Worker Result Delivery

- Recording a canonical worker result atomically records a pending parent
  notification. `reported` means stored, not received by the model.
- The notification contains the complete typed result, child identity, evidence
  ID, and repository/completion metadata. No raw worker transcript is forwarded.
- Notifications wait in SQLite while the parent is busy or disconnected, and
  use its ordinary turn-boundary queue rather than a concurrent `send_turn`.
- A failed send remains retryable. Repeated sweeps or duplicate result reports
  do not create duplicate parent turns. A restart before queueing does not lose
  the result. An ambiguous in-flight write is surfaced rather than silently
  treated as delivered.
- Direct-agent proxy sessions retain their no-parent-notification behavior.
- Peeking at a completed owned worker returns its canonical typed result rather
  than claiming the parent has already received it. Foreign workers stay hidden.
- Verify with synthetic runtimes only; no workers or authenticated provider
  sessions may be launched for this task.
