# Test contract — unified Memory surface (`feat/memory-unified-redesign`)

Supersedes `feat-memory-core.md`. The separate full-screen, dark-only "Memory
Core" constellation is removed: one product gets one surface. The Memory dialog
(Graphite & Paper chrome, achromatic — no `mc-*` token family, no green accent,
no confidence-sized dots) absorbs the useful aggregations the constellation
carried: per-record recall counts, the 14-day recall series, the packet-budget
meter, and the consolidation log. Everything below is exercised by `bun run dev`
and the colocated Vitest files.

## Pure logic — `src/memoryStats.ts` (`memoryStats.test.ts`, node env)

Renamed from `src/memoryCore.ts`; the graph layer (`layoutConstellation`,
`buildEdges`, `nodeRadius`, `deriveCoRecall`, `GraphNode`/`GraphEdge`) is gone.

- `recordState(record)` maps a record to exactly one of
  `pinned | active | proposed | superseded | tombstoned`: active+`user_explicit`
  → `pinned`; active+`model_proposal` → `active`; `proposed` → `proposed`;
  `superseded` → `superseded`; `deleted` → `tombstoned`.
- `deriveRecallStats(records, audit)` aggregates the mock audit into per-record
  recall counts, last-recalled day, in-packet ratio, a 14-day daily series, plus
  `injectionsPerDay` and `budgetCharsUsed ≤ budgetCharsMax` (4000). Empty audit
  → all zeros, no throw.
- `searchRecords(records, query)` is case-insensitive substring match over
  `body`+`kind`; an empty query returns every record; no match returns `[]`.

## Surface — `src/components/MemoryDialog.tsx` (`MemoryDialog.test.tsx`, jsdom env)

Existing contract (`feat-memory-ui.md`, `feat-memory-ui-fields.md`) still holds:
save/supersede/forget round-trips, the review queue, extraction settings, the
injection toggle, the body cap. New on top:

- A search input on the pins tab filters the visible rows via `searchRecords`;
  clearing it restores the full list.
- A pin row whose record has recalls in `memoryRecallStats` shows a recall meta
  line (`recalled N× · last Nd ago`); a record with zero recalls shows none.
- An **Activity** tab renders: the 14-day injection sparkline, the packet-budget
  meter (`budgetCharsUsed / budgetCharsMax`), and the consolidation log entries
  with their op vocabulary. All achromatic — no `mc-*` classes anywhere.
- Recall stats and the consolidation log load with the other reads behind the
  same read generation; a slow earlier read never lands over a fresher one.

## Removal — no second memory surface

- `src/components/MemoryCore.tsx` and its test are deleted; `AppModal` loses
  `"memory-core"`; `BridgeSidebar` has exactly one memory row ("Memory"), and
  `App` renders exactly one memory surface.
- `src/index.css` carries no `--color-mc-*` / `--radius-mc-*` tokens and no
  `.mc-hero` / `.mc-grid` rules; `AGENTS.md` no longer documents a separate
  dark-only memory surface.
- `bridgeApi.memoryGraphRecords` and `bridgeApi.memoryCoRecallPairs` are gone;
  `memoryRecallStats` and `memoryConsolidationLog` remain and keep their
  scope-required guard.

## Gates

- `bun run check`, `bun run test`, `bun run build` all green.
- No ad-hoc hex values: every color in the dialog resolves to an existing
  Graphite & Paper token through a Tailwind utility.
