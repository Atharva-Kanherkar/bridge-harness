# Contract: unread state, sidebar dots and the dock badge

Branch `feat/attention-unread`, stacked on `feat/attention-escalation`.
Implements the first half of #908, slice 4 of #904. Locked before
implementation. The menu bar count is the second half, in its own PR.

## What this changes

A missed notice today lives only as long as its toast. After this, a chat
that finished, failed or needs you while you were elsewhere stays marked in
the sidebar and counted on the dock icon until it is opened, on any client.

## Functional behavior

Core, `attention::unread_summary(db)`:

- A session's own unread kind is the kind of its newest ledger row when that
  row is unseen, unresolved and of a known kind. A superseded row never
  counts. A seen or resolved newest row means nothing is unread there.
- Only sessions that exist and are not archived count. An archived worker
  rolls up nothing, and nothing rolls up into an archived chat.
- Rollup: each top-level chat (no `parent_session_id`) takes the strongest
  unread kind across itself and every non-archived descendant, at any depth.
  Strength: `needsYou` > `escalation` > `failed` > `finished`.
- Result:
  - `byChat`: top-level chat id -> rolled-up kind. These are the sidebar rows.
  - `bySession`: session id -> its own unread kind, workers included.
  - `total`: `byChat` entries, so the badge equals the marked sidebar rows.
  - `needsYou`: `byChat` entries whose kind is `needsYou` or `escalation`,
    the ones that wait on the person to act.
- Read-only. It never changes the ledger or its revision.

Protocol 1.26: `attention/unread_summary -> { total, needsYou, byChat,
bySession }`. It takes no params, like the other parameterless reads
(`get_recall_stats`, `get_attribution_settings`). A 1.25 client still
pairs with a 1.26 daemon. A 1.26 client needs a 1.26 daemon, because the
method is new.

Desktop:

- `useUnreadSummary(sessionsKey)` reads the summary on mount, on every
  `attention/changed`, and when `sessionsKey` changes (the visible session
  ids, so an archive or delete is reflected without a ledger change). A
  response that arrives after a newer request was sent is dropped. A failed
  read keeps the last summary.
- Sidebar: a row in `byChat` shows an unread dot before its title and the
  title in medium weight, and its accessible name starts with "Unread". The
  dot is achromatic (`bg-foreground`).
- When the rolled-up kind is `needsYou` and the row's own status line would
  not already say so (an orchestrator idle while its worker waits), the
  status line reads "needs you" with the warning dot.
- The selected chat in the workspace view, while the window is visible and
  focused, is marked seen whenever the summary lists it in `bySession`, and
  again when the window regains focus. So a chat you are looking at never
  stays unread, and one that finished while Bridge was in the background is
  cleared when you come back to it.
- Dock badge: the app sends `total` to the shell over the
  `bridge-unread-badge` event whenever it changes. The shell sets the main
  window's badge count, and clears it at zero. Outside Tauri this is a no-op.
- Mock mode computes the same summary from the mock ledger and the mock
  sessions' parent links.

Out of scope: the menu bar count (second half of #908), toast suppression for
the chat on screen and per-kind settings (#909), Mission Control grid tiles.

## Unit tests (bridge-core)

- `attention::tests::unread_is_the_newest_unseen_unresolved_row`
- `attention::tests::seen_resolved_and_superseded_rows_are_not_unread`
- `attention::tests::a_worker_request_rolls_up_to_its_top_level_chat`
  (a depth-2 worker too)
- `attention::tests::the_strongest_kind_wins_the_rollup`
- `attention::tests::archived_sessions_are_not_unread`
- `attention::tests::unread_summary_leaves_the_revision_alone`

## Integration / functional tests

- bridged dispatch: `attention/unread_summary` on an empty ledger returns
  `{ total: 0, needsYou: 0, byChat: {}, bySession: {} }`, and after a
  finished turn counts that chat.
- Shell: `parse_unread_badge` maps 0 to no badge, n to n, and garbage to no
  change.
- Protocol gates as in the earlier slices (artifacts, naming, strictness,
  registry, signatures).

## Frontend tests

- `attentionMock.test.ts`: the mock summary rolls a worker up, prefers the
  stronger kind, and skips seen and resolved rows.
- `useUnreadSummary.test.tsx`: reads on mount, rereads on a hint and on a new
  `sessionsKey`, drops a stale response, keeps the last summary on failure.
- `BridgeSidebar.test.tsx`: an unread row has the dot and the "Unread"
  name; a rolled-up `needsYou` reads "needs you"; a read row has neither.
- `App.attention.test.tsx`:
  - A chat that finishes while another is open shows the dot, and the badge
    event carries 1.
  - Opening it clears the dot and sends 0.
  - Marking it seen from another client (a direct `markAttentionSeen`)
    clears the dot.
  - A worker's request marks its orchestrator's row "needs you".
  - A row for the chat on screen in a focused window is marked seen at once.

## Smoke tests

- `cargo test -p bridge-core attention::`
- `cargo test -p bridge-protocol`, `cargo test -p bridged dispatch`
- `bun run check`, `bun run build`, `bun run test`

## E2E / Manual

Live daemon from this branch with its own data directory:

1. `bridge exec --method attention/unread_summary` returns the empty summary.
2. Finish a turn in a chat: the summary has it in `byChat` and `total` 1.
3. `attention/mark_attention_seen` for it: `total` 0.

Mock mode in a browser: push a `finished` row for a chat that is not open,
see the dot; open the chat, the dot goes. Screenshots dark and light.

The dock badge itself needs a release bundle; launching one next to the
daily app risks its data directory, so it is checked by the shell unit test
and left as a manual step for the reviewer.
