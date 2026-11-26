# Contract: Screen Time breakdown (projects, pull requests, hours, harnesses)

Branch `feat/screen-time-breakdown`. Locked before implementation. Plan note:
`bridge://p/01a2168c-ba9f-7d20-892d-6cf087868082`. Builds on
`testing/feat-usage-screen-time.md`; every rule there still holds.

The Time tab says how long. This adds where the time went and what came of
it: which project, which repository, how many pull requests, which hours and
which harnesses. Everything stays local.

## Definitions

1. A chat's **project** is `sessions.workspace_id -> workspaces.project_id ->
   projects(id, name)`. A chat with no workspace, a workspace with no
   project, or a project row that is gone belongs to **No project**
   (`projectId` null).
2. **Agent time per project** uses the agent-time rules unchanged (counted
   kinds, per-chat minutes, gap fill of 2, chats add up), restricted to
   minutes whose local day is in the range, and grouped by the chat's
   project. The project rows' agent seconds add up to the range's agent total.
3. **Your time per project**: each minute of your time may also name the
   chat that was open in the window then. The minute counts toward that
   chat's project. Minutes with no chat open (Settings, Usage, other
   screens) count toward your total only. Recorded from this change on;
   earlier minutes have no chat.
4. **Chats per project** is the number of distinct top-level chats
   (`parent_session_id` null) of that project with at least one counted
   agent minute in the range. Workers and harness subagents add time but are
   not chats.
5. **Pull requests opened** are rows of `session_pull_requests` with
   attribution `toolCompletion` (a `gh pr create` the agent ran) whose
   `created_at` falls on a local day in the range. One pull request is one
   `(repo_host, repo_owner, repo_name, number)`, however many chats link it.
   Its opened day and project come from its earliest row; its title, URL and
   state from the row fetched most recently. State is the snapshot's `state`
   (`open`, `merged`, `closed`), else `unknown`. Manually attached pull
   requests never count.
6. **Hours**: 24 entries, hour 0 to 23 in the caller's zone, each holding
   your seconds and agent seconds over the range's minutes in that hour.
7. **Harnesses**: agent seconds in the range per `sessions.harness`, with the
   number of chats (rule 4) per harness. Harnesses with zero seconds are left
   out.

## Wire (protocol 1.61)

8. `usage/record_presence` params gain optional `chats: [{ sessionId,
   minutes }]`: minutes from `minutes` that were spent with that chat open.
   A chat minute is stored only when the same minute is accepted from
   `minutes` in this call. A minute keeps the first chat stored for it.
   More than 1,440 chat minutes in one call, an empty session id, or one
   over 128 characters is `Invalid`. Omitting `chats` behaves exactly as
   before.
9. `usage/screen_time` results gain:
   - `projects`: `[{ projectId, name, humanSeconds, agentSeconds, chats,
     pullRequests }]` for projects with any time or pull request in the
     range, sorted by your plus agent seconds descending, then name.
   - `pullRequests`: `{ opened, merged, open, closed, repos: [{ repo,
     opened, merged }], recent: [{ repo, number, title, url, state,
     openedDay }] }`. `repo` is `owner/name`. Repos sorted by opened
     descending, then name. `recent` holds at most 6, newest first.
   - `hours`: always 24 entries, rule 6.
   - `harnesses`: `[{ harness, agentSeconds, chats }]`, descending.
10. Protocol goes 1.60 to 1.61. A 1.60 client still pairs with a 1.61
    daemon; a 1.61 client needs a 1.61 daemon (it sends `chats`).
    Artifacts are regenerated, not edited.

## Storage (migration 100)

11. `human_activity_chats(minute INTEGER PRIMARY KEY, session_id TEXT NOT
    NULL)`, written with `INSERT OR IGNORE`. No other table changes.

## Tracker

12. At each tick that counts a minute, the tracker also notes the chat open
    in the window (the selected chat while the chat view shows), or none. A
    minute's chat is the one seen at its last counting tick. A flush sends
    the minutes and, grouped by chat, the minutes that had one. A failed
    send keeps both.

## Time tab

13. Under the daily chart: **Projects** and **Pull requests** side by side
    on wide windows, then **When you work** (hours) and **Agents by
    harness**.
14. Projects: one row per project with a two-part bar (you, agents) scaled
    to the largest row, your time, agent time, chats and pull requests. The
    first 6 rows show; `Show all N` reveals the rest. No project reads `No
    project`. A note says your time per project counts from this release.
    Empty: `No project time in this range.`
15. Pull requests: the opened count, then merged, open and closed counts, a
    bar per repository, and the recent list linking to each pull request
    with its state. Merged uses the merged colour; nothing else is coloured.
    Empty: `No pull requests opened from Bridge in this range.`
16. When you work: 24 columns, you and agents side by side, labelled at
    12a, 6a, 12p, 6p, with the busiest hour of yours named (agents' when you
    have none). Hover or arrow keys read one hour.
17. Agents by harness: harness mark, name, a bar of its share of agent time,
    time and chats.
18. Mock mode serves all four so `bun run dev` shows them.

## Tests

- Rust (`bridge-core screen_time::`): project grouping including No
  project and gap fill; project agent seconds sum to the range total; your
  time per project and the minutes with no chat; chats count top-level only;
  PR dedupe, manual excluded, state from the latest fetch, local-day range,
  recent cap and order; hours in a non-UTC zone; harness split; presence
  chats stored only alongside accepted minutes, first chat kept, validation.
- Protocol: `cargo test -p bridge-protocol` (strictness, artifacts,
  handshake 1.60 to 1.61).
- Vitest: tracker sends chat minutes grouped and keeps them on failure; the
  tab renders all four cards, the Show all toggle, the empty states and the
  PR links.
- `bun run build`, `bun run check`.
