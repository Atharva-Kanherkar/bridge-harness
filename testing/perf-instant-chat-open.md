# perf/instant-chat-open: test contract

Opening a long chat must paint at once, and the open chat must not keep
downloading its whole history while other chats work. Written before the
implementation; it is the definition of done for the branch.

## Measured problem

Release build, read-only against the live database, machine load 35 to 45.
One `sessions/get_session_forest` call:

| Chat | Entries | Frame | Snapshot build |
| --- | --- | --- | --- |
| A | 10,254 | 56 MB | 4.8 to 6.6 s |
| B | 5,948 | 59 MB | 5.5 s |
| C | 29,101 | 40 MB | 3.6 to 5.1 s |

- `usage` was 30.7 MB (35,172 rows) in every chat of the busiest project. Its
  only reader, `turnBudget` in `EnvPanel`, is never mounted.
- The entry window walked the whole active branch twice (count, then sort and
  take the newest 1,500) and re-parsed an over-budget window at every rung.
- `branch_leaves` loaded and parsed every entry of the chat to find childless
  ones (229 MB of payload in the largest chat).
- The forest digest included global maxima (`events`, `usage_ledger`, and
  global lease, runtime, queue and adoption counts), so any activity anywhere
  refetched the open chat's full snapshot on the next 3 s poll.
- `ForestCache` refused snapshots over about 8 MB of JSON and stringified every
  snapshot it stored.

## Functional behavior

### 1. Bounded branch walk

- The window is the newest `limit` ancestors of the active head, oldest first,
  exactly as before, for chains and for forked histories.
- The recursive walk stops after `limit` rows.
- `total` equals the full active-branch length. On a single chain (one root,
  one leaf) it comes from an indexed count; any other shape keeps the exact
  recursive count.
- A head moved back along a chain (no fork yet) counts only its ancestors.
- Trimming behavior and the budget ladder are unchanged.

### 2. Leaves by SQL

- `branch_leaves` returns the same entries in the same order as before, without
  loading any non-leaf payload.

### 3. Usage diet

- The snapshot's `usage` holds only this chat's rows for its active turn, or
  its newest recorded turn when no turn is active. `turnBudget` returns the
  same result for this chat as before.

### 4. Scoped digest

- Unchanged when another project's usage, events, leases, runtimes, queue or
  adoptions change, and when another chat in a different project appends.
- Still moves for: this chat's entry appends, head moves, status and active
  turn, this chat's usage rows, this project's leases, runtimes, queue,
  adoptions and reason events, and completion attempts.

### 5. First page

- `sessions/get_session_forest` accepts optional `limit`, clamped to 1..1,500.
  Omitted means 1,500. `entryWindow.returned` reports the page size and
  `entryWindow.total` the full branch.

### 6. Open path (App)

- A cached snapshot paints on the selection commit.
- A miss fetches a first page, paints it, then fetches the full window without
  waiting for the 3 s poll.
- The "showing the most recent N" notice is hidden while only the first page is
  loaded.
- A reopen whose digest matches the cached digest does not refetch the snapshot.

### 7. Older rows arriving above

- When older rows arrive above the mounted rows for the same chat, the mounted
  rows stay mounted (no remount), the window start shifts by the number of
  rows that arrived, and the rest hydrates in idle slices as before.
- If the previous first row is gone from the new list, the window reseeds on
  the newest rows.

### 7b. Background tasks without reading history

Profiling the first page after items 1 to 5 showed it was still spent in
`background_activity`: every snapshot, provider start and exit JSON-parsed
every tool and command payload of the chat (239 MB in one chat), and every
store open did it for the whole database (3.3 GB, 9.4 s).

- Migration 76 adds `session_background_tasks`, one row per task holding its
  latest background snapshot, kept by an insert trigger and cleared by a
  delete trigger. The migration backfills tasks still running, so the first
  open after upgrade closes exactly what the old scan would have.
- `current` and `interrupt` read that table and re-check the entry's own
  status. Results, order and the 256 cap are unchanged.
- Ordinary tool entries never get a row.

### 8. Cache

- A snapshot over the cache's byte budget is stored as its newest page instead
  of being dropped, and is marked partial so the open path fetches the full
  window.
- Size is estimated once per snapshot, not by stringifying on every refresh.
- Count and byte limits still hold.

## Tests

- store: bounded window equals the old full walk on a chain, a rewound chain
  and a fork; `total` matches; budget ladder tests still pass.
- session_forest: SQL leaves equal the old in-memory leaves on chain and fork.
- sessions: snapshot `usage` holds only the chat's latest turn; digest stable
  under another project's activity, moves under this chat's and this
  project's.
- protocol: `limit` decodes, defaults, clamps; artifacts regenerated.
- background_activity: the index follows appends, completions, per-session
  interrupts and deletes; an upgraded database closes tasks left running
  before the index existed; existing projection and recovery tests unchanged.
- frontend: transcript window shift keeps mounted rows; reseed when the anchor
  is gone; cache stores a page for an oversized snapshot and marks it partial;
  App opens a miss with a page then the full window, and hides the notice
  while partial.
- `bun run build` and `bun run test` green.
