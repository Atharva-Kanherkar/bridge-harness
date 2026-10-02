# Test contract: live transcript gap replay

Locked before implementation.

## Incident

An orchestrator chat on a 14k-entry session showed the user's messages and one
settled command group, but none of four assistant replies. Every turn had
completed and was persisted (`session_entries` 14264–14306). The UI was still
drawing a forest snapshot that ended at entry 14261, the live window had lost
the `message.completed` frames, and the optimistic rows it never reconciled
kept "Thinking" on screen.

Two defects let that happen:

1. The webview has no gap recovery. The daemon drops live frames when a
   connection lags and says so with `stream-lagged`, but the webview only
   reconciles terminals on that marker. A durable frame that goes missing
   stays missing until a full forest refetch lands. On this session that
   refetch is 5.4 MB and takes 3–9 s, so under load it lagged or never landed.
   `sessions/replay_session_events` returns the same missing frames in 0.06 s.
2. The shell subscribed on invoke lane 0. The daemon writes every frame on a
   connection under one writer lock, so a 10.8 MB `get_state` response on lane
   0 stalls the live stream behind it until the 1024-frame sink overflows and
   frames are dropped.

## Expectations

### Frontend: `src/liveReplay.ts`

- A durable frame whose sequence skips ahead of the session's cursor replays
  the missing range from that cursor, via `replay_session_events`, and
  delivers the replayed frames.
- Contiguous frames never trigger a replay.
- Frames at or below the cursor (duplicates, replayed frames) never trigger
  a replay.
- A session with no cursor adopts the first durable frame it sees. There is
  no gap to detect before anything is known.
- A forest snapshot seeds the cursor with its newest sequence. Seeding never
  lowers a cursor.
- Replay pages until a short page, so a gap wider than one page is filled.
- A gap seen while a replay for that session is in flight does not start a
  second concurrent replay. It runs one more pass after the first settles.
- A lag marker replays every recently active session from its cursor, capped
  to the most recent few.
- A failed replay does not throw into the event listener. The next gap or
  lag marker retries.

### Frontend: `src/agentEvents.ts`

- A transient frame is anchored to the newest durable sequence of **its own
  session**, not the last durable frame of any session.
- A replayed batch of older frames does not lower a session's anchor.

### Frontend: `App.tsx` wiring

- Every live frame goes through the replayer. Replayed frames go into the
  same bounded live window and are deduplicated by id.
- The selected session's forest poll seeds the replayer.
- `stream-lagged` triggers a lag replay and forces the next forest poll to
  refetch instead of trusting the digest.

### Shell: `src-tauri/src/daemon_host.rs`

- The notification subscription gets its own daemon connection when the
  daemon grants one. That connection never serves invokes.
- With only one connection available, the shell still subscribes and serves
  invokes on it, as before.

## Regression proof

A unit test replays the incident shape: forest ends at 14263, live delivers
14264 and then 14271, and the replayer fetches after 14264 and delivers the
lost assistant reply.

## Gates

`bun run build`, `bun run test`.
