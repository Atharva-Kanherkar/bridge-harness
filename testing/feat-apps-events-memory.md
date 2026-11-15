# feat/apps-events-memory: Test Contract

Apps slice 2, PR B (issue #990, locked 2026-11-15; PR A shipped in #1000).
Apps write typed `app.event` entries into the session forest, and memory
extraction reads them only inside the memory scope each app's manifest
declares. Plan note: `bridge://p/01a1debb-7a02-7e03-a486-610b9c62daca`.

No protocol method, no protocol version bump, no migration.

## Functional Behavior

### The entry

- Kind `app.event`, `context_visibility = 'hidden'`, payload
  `{appId, type, summary, payload}` (plus the store's `_bridgeRepoState`).
- Hidden means it never enters context projection and is never an FTS recall
  hit, like `turn.started`.

### The writer: `apps::events::record_tx(tx, session_id, app_id, event_type, summary, payload)`

- Returns `Ok(Some(entry_id))` after appending the entry in the caller's
  transaction.
- Refuses with Bridge's `invalid` error, writing nothing:
  - an unknown app id: "Bridge has no app called <id>."
  - a type the manifest does not declare: "<Name> does not record <type> events."
  - an empty summary, or one over 280 characters
  - a payload that is not a JSON object, or whose JSON is over 4096 bytes
- An app that is off returns `Ok(None)` and writes nothing. Off is a normal
  state, so it must not fail the caller's transaction.
- Every string in the summary and the payload (recursively) passes through
  `secret_interception::intercept(..).sanitized.text` before it is written.

### Outgoing manifest

- `events: ["outgoing.sent"]`,
  `memory: ["the people, channels and repositories the person writes to"]`.
- Every other built-in keeps empty `events` and `memory`.

### First emitter: Outgoing send

- A successful send appends `outgoing.sent` (unchanged: visibility
  `excluded`) and then, in the same transaction, one `app.event` of type
  `outgoing.sent` whose parent is the `outgoing.sent` entry.
- Summary: Slack "Sent a Slack message to <label>" (thread: "Replied in a
  Slack thread in <label>"); GitHub "Posted a GitHub comment on
  <repo>#<number>". Payload `{family, destination}`. The message text is never
  in the event.
- With Outgoing off, only `outgoing.sent` is written. A refusal from the
  writer is logged and `outgoing.sent` still commits.
- A failed send writes neither (unchanged).

### Digest

- `memory_extraction::build_digest` appends, after `Conversation:`, an
  `App activity` block built from the newest 20 `app.event` entries of the
  session, oldest first, grouped per app:
  `<App name> (memory may keep only: <scope lines joined by "; ">)` then one
  `[entry-id] <summary>` line per event.
- Entries of an app that is off, unknown, or has an empty `memory` scope are
  left out. If nothing is left, there is no block.
- A session with app activity and no messages still yields a digest.
- The block counts against the existing `MAX_DIGEST_CHARS` budget; the
  conversation is not shortened to make room for it.

### Prompt

- `extraction_instructions` gains one paragraph: activity lines record what
  happened through an app, the person did not say them; propose a durable
  pattern only inside that app's stated scope; never propose from a single
  one-off event; cite the activity line's id. The rationale rule names
  "the user.message or app activity line".

### Gate

- A proposal whose rationale cites an `app.event` id is written as
  `proposed`, never auto-applied, even in auto-apply mode with high
  confidence (it is not a cited `user.message`).

### Frontend

- `normalizeSessionEntry` returns `null` for `app.event`, so a reloaded chat
  never shows "Unrecognized event: app.event".

## Unit Tests

Rust, bridge-core:

- `apps::events::tests::records_a_declared_event_as_hidden`
- `apps::events::tests::refuses_undeclared_types_and_unknown_apps`
- `apps::events::tests::refuses_oversized_summary_and_payload`
- `apps::events::tests::an_off_app_writes_nothing`
- `apps::events::tests::secrets_are_scrubbed_from_summary_and_payload`
- `apps::builtin::tests::builtins_validate_and_list_twelve_apps_in_order` (still green with Outgoing's events and scope)
- `outgoing::send` test: a send writes `outgoing.sent` and one `app.event`; with Outgoing off only `outgoing.sent`
- `memory_extraction::tests::digest_lists_app_activity_under_its_scope`
- `memory_extraction::tests::digest_skips_off_and_unscoped_apps`
- `memory_extraction::tests::digest_caps_app_activity_at_twenty`
- `memory_extraction::tests::app_activity_alone_yields_a_digest`
- `memory_extraction::tests::a_proposal_citing_app_activity_is_never_auto_applied`
- a context projection / recall test: `app.event` is never projected and never a recall hit

Vitest:

- `src/transcript/codec.test.ts`: `app.event` normalizes to `null`

## Integration / Functional Tests

- The Outgoing send test above runs the real `send` path with a stubbed
  executor and reads the forest rows.

## Smoke Tests

- `bun run build` and `bun run test` green.
- `cargo test -p bridge-core` green.

## E2E Tests

N/A: no new UI. The eval below is the behavioral test of the prompt change.

## Eval

- `testing/evals/memory-extraction/cases.json`: optional per-case
  `appEvents: [{app, type, summary}]`, inserted by `memory_evals.rs` as
  `app.event` entries (visibility `hidden`).
- New cases: an in-scope repeated destination is proposed; an out-of-scope
  detail in a summary is ignored; a single one-off send is ignored; an
  empty-scope app's activity never reaches the model.
- `BRIDGE_MEMORY_EVAL=1 cargo test -p bridge-core --lib memory_evals::live_extraction -- --ignored --nocapture`
  on main's prompt (with the new cases) and on this branch. Output pasted
  unedited in the PR. The 12 existing cases must not regress; forbidden
  proposals stay at 0.

## Manual / cURL Tests

- On a short data dir with a chat that has an Outgoing draft and a configured
  Slack or GitHub destination: send, then
  `sqlite3 bridge.db "SELECT kind, context_visibility, json_extract(payload,'$.summary') FROM session_entries WHERE kind IN ('outgoing.sent','app.event')"`.
  If no destination is configured on this machine, say so in the PR.
