# Contract: Screen Time (your time, agent time, all-time total)

Branch `feat/usage-screen-time`. Locked before implementation. Plan note:
`bridge://p/01a1fd85-a250-79c0-bcff-c30f57318831`.

A Time tab on the Usage screen that answers two questions, the way Apple's
Screen Time does for a phone: how much of my day did Bridge take, and how much
work did my agents do in that time. Everything is local; nothing is synced.

## Definitions

1. **Your time** is whole minutes in which the Bridge window was focused,
   visible, and you had touched it (pointer, key, wheel, touch, or focusing
   the window) within the last 5 minutes. A minute counts once however many
   windows report it. Seconds = minutes x 60.
2. **Agent time** is whole minutes in which a counted chat wrote any session
   entry other than `user.message`, per chat, added up across chats (two
   chats working through the same minute give two minutes). Within one chat a
   gap of at most 2 silent minutes between active minutes counts as active,
   so a long quiet tool run is not lost; a longer gap is idle.
3. **Counted chats** are sessions whose `kind` is null, `direct`,
   `orchestrator` (including workers) or `harness_subagent`. Briefings,
   suggestions, extraction, connector runs, chat search, imported sessions
   and every other kind never count.
4. **Days** are calendar days in the caller's IANA time zone. A minute
   belongs to the local day its start falls in.

## Storage (migration 88)

5. `human_activity_minutes(minute INTEGER PRIMARY KEY)`: epoch minutes, UTC.
6. `agent_activity_minutes(session_id, minute, PRIMARY KEY(session_id, minute)) WITHOUT ROWID`.
7. A trigger on `session_entries` insert (`kind <> 'user.message'`) adds the
   entry's minute with `INSERT OR IGNORE`. No other write path changes.
8. Entries that existed before the migration are counted by a backfill that
   never runs on boot and never holds the core database mutex while reading:
   it reads through its own read-only connection in rowid batches up to the
   highest rowid at migration time, writes each batch in one short
   transaction, and records its cursor so a restart resumes. It starts the
   first time `usage/screen_time` is asked and runs at most once per database
   at a time. Backfilled minutes equal what the trigger would have written.

## Wire

9. `usage/record_presence` `{ minutes: number[] }` -> `{ recorded: number }`.
   Minutes in the future (beyond the current minute) or older than 7 days are
   dropped, not errors. More than 1,440 minutes in one call is `Invalid`.
   `recorded` is the count of minutes newly stored; repeats store nothing.
10. `usage/screen_time` `{ sinceDay, untilDay, timeZone? }` -> result with
    `days` (dense: one entry per day from `sinceDay` to `untilDay`, zeros
    included, each `{ day, humanSeconds, agentSeconds }`), `allTime`
    (`{ humanSeconds, agentSeconds, humanSinceDay, agentSinceDay }`, the
    since-days null when nothing is recorded), and `agentHistory`
    (`complete` | `counting`). A range over 92 days, an inverted range, a bad
    day, or an unknown zone is `Invalid`. A missing zone is UTC.
11. Params structs deny unknown fields. Artifacts are regenerated, not edited.

## Tracker (webview)

12. Mounted once in the app. Every 15 s it checks: tracking enabled, document
    visible, window focused, input within 5 minutes. If all hold, the current
    epoch minute joins a pending set.
13. Pending minutes are sent every 60 s and immediately on blur, on the
    document turning hidden, and on `pagehide`. A failed send keeps them for
    the next flush, capped at 1,440 (oldest dropped first).
14. `bridge.screenTime.v1` in `localStorage` holds `{ track: boolean }`,
    default `true`; a malformed value falls back to the default. With
    tracking off nothing is recorded or sent, and pending minutes are dropped.

## Time tab

15. The Usage heading's View control reads `Usage | Insights | Time`.
16. A `Week | Month` control picks the last 7 or 30 local days ending today.
17. Summary: **Today** (your time today, with agent time beside it),
    **Daily average** (your time over the days in the range), the range
    **total**, and **All time** (your cumulative time since tracking began,
    with "since <day>"). Agent time shows the same total and how many minutes
    of agent work each minute of yours bought (`3.2×`), only when your time in
    the range is above zero.
18. Chart: one column per day, your time and agent time side by side,
    achromatic. Hover or arrow keys read one day (date, your time, agent
    time). An empty range says `No time recorded in this range.`
19. Cumulative chart: your running all-time total at the end of each day in
    the range (the all-time total minus the range, then adding each day).
20. While `agentHistory` is `counting`, a note says earlier agent time is
    still being counted, and the tab refetches every 3 s until `complete`.
21. A `Track my time` switch writes the preference. When off, a note says
    your time is not being recorded and agent time still counts.
22. Durations format as `0m`, `45m`, `3h 05m`, and `1,204h` past 100 hours.
23. Mock mode serves deterministic data so `bun run dev` shows the tab.

## Tests

- Rust (`bridge-core screen_time::`): trigger writes minutes and skips
  `user.message`; hidden kinds excluded; gap fill at 2 and 3 silent minutes;
  parallel chats add; local-day bucketing in a non-UTC zone across midnight;
  dense days; all-time and since-days; backfill output equals trigger output
  and resumes from its cursor; presence drops future and stale minutes, is
  idempotent, rejects 1,441 minutes; range and zone validation.
- Protocol: `cargo test -p bridge-protocol` (registry, strictness, artifacts).
- Vitest: tracker rules 12 to 14 with fake timers; formatting rule 22; the
  tab renders summary, chart, cumulative, counting note and polling, empty
  state, and the switch.
- `bun run build`, `bunx tsc -b`.
