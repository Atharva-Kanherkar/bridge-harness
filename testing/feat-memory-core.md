# Test contract — Memory Core surface (`feat/memory-core`)

Issue #416. A full-screen, dark-only memory-graph surface built on the Geist-derived
primitives documented in `AGENTS.md` ("Memory Core surface — design primitives").
This slice ships the **frontend surface** on the `src/api.ts` mock/derived layer;
the protocol-first `memory.recall_stats` / `memory.co_recall_pairs` Rust+daemon
implementation is a tracked follow-up (see PR body). Everything below is exercised
by `bun run dev` and the colocated Vitest files.

## Pure logic — `src/memoryCore.ts` (`memoryCore.test.ts`, node env)

- `nodeState(record)` maps a record to exactly one of `pinned | active | proposed | superseded | tombstoned`:
  active+`user_explicit` → `pinned`; active+`model_proposal` → `active`; `proposed` → `proposed`;
  `superseded` → `superseded`; `deleted` → `tombstoned`.
- `nodeRadius(confidenceBps)` is monotonic in confidence and clamped to `[MIN_R, MAX_R]`;
  a null confidence yields the mid radius, never `NaN`.
- `layoutConstellation(records, w, h)` is **deterministic** (no `Math.random`): the same input
  yields identical coordinates, every node sits inside `[0,w]×[0,h]`, and no two nodes share a point.
- `buildEdges(records, coRecall)` emits a `supersedes` edge for every record with a `supersedes`
  parent present in the set, a `conflict` edge between every pair sharing a `conflictGroup`, and a
  `corecall` edge per co-recall pair; it never references an id outside the record set.
- `deriveRecallStats(records, audit)` aggregates the mock audit into per-record recall counts,
  last-recalled day, in-packet ratio (`recalls / injectionCount`), a 14-day daily series, plus
  `injectionsPerDay` and `budgetCharsUsed ≤ budgetCharsMax` (4000). Empty audit → all zeros, no throw.
- `searchRecords(records, query)` is case-insensitive substring match over `body`+`kind`; an empty
  query returns every record; no match returns `[]`.

## Surface — `src/components/MemoryCore.tsx` (`MemoryCore.test.tsx`, jsdom env)

- Closed (`open={false}`) renders nothing and holds no state.
- Open loads active + proposed records under `account:local` and refetches on `memory-changed`
  behind a read generation (a slow earlier read never lands over a fresher one).
- The constellation renders one `<circle data-node>` per active/proposed record with a
  `data-state` attribute; pinned nodes carry a ring element and proposed nodes a dashed stroke —
  identity is never color-alone (state is also on `data-state` + shape).
- Selecting a node populates the Inspector with the record's body, kind, provenance, confidence,
  lineage (`supersedes` chain), and a recall sparkline; Edit round-trips through
  `supersedeMemoryRecord` and Forget through `deleteMemoryRecord`.
- The Review queue lists proposed records and Accept calls `approveMemoryRecord`, Reject calls
  `rejectMemoryRecord`; an accepted record leaves the queue and appears in the graph.
- The Search box filters the visible node set via `searchRecords`.
- `Escape` closes the surface.

## Wiring

- `AppModal` gains `"memory-core"`; `BridgeSidebar` gains a "Memory Core" action row that calls
  `onOpenMemoryCore`; `App` renders `<MemoryCore open={modal === "memory-core"} />`. Reachable from a
  fresh install with no dev flags (end-to-end reachability gate).

## Gates

- `bun run check`, `bun run test`, `bun run build` all green.
- No ad-hoc hex values in `MemoryCore.tsx`: every color resolves to a `--color-mc-*` `@theme` token
  or an existing token, used through a Tailwind utility.
