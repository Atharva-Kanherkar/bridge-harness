# Test contract: one skeleton kit for every loading state

Branch `feat/unified-loading-skeletons`.

## What must hold

1. `src/components/ui/skeleton.tsx` is the only file that names `.skeleton-shimmer` or `.animate-skeleton-in`.
2. No class string in `src/` pairs `animate-pulse` with a `bg-muted*` fill.
3. A skeleton region is `role="status"`, `aria-busy="true"`, `aria-label={label}`, and carries a `sr-only` copy of the label. Every bar is `aria-hidden`.
4. The `sr-only` label is the region's first child, so a `divide-y` region never draws a rule under its last row.
5. Bars in one row share a stagger delay; the delay steps 90ms per row and stops at the eighth row.
6. `SkeletonRows` without a label renders no status region, so it can sit inside a `SkeletonRegion` without announcing twice.
7. A caller's `className` reshapes a bar (height, radius, width) without dropping the shimmer.
8. Existing accessible names and test ids survive the migration: `Loading conversation`, `Loading pull requests`, `Opening pull request #…`, `memory-loading`, `plugin-catalog-skeleton`, `Loading usage`, `Loading meter`, `Loading models…`, `Loading activity`.
9. Reloads keep the last data: `WorktreeStorage` shows its skeleton only while it has no inventory.

## Where it is tested

- 1, 2: `src/designSystem.test.ts` "draws every loading placeholder through the skeleton kit". Verified to fail on a probe file holding `animate-pulse rounded bg-muted` and `skeleton-shimmer`.
- 3 to 7: `src/components/ui/skeleton.test.tsx`.
- 8: the existing loading tests in `AgentConversation`, `AgentConversation.history`, `GitHubPane`, `MarketplaceScreen`, `MemoryDialog`, `UsageDot`, `UsageDot.interaction`, `UsageScreen`, `MeterPopover`, `ChatModelControl`, `App`, `WorkView`, unchanged.

## Out of scope

`settings/StoragePage.tsx` (reworked in its own PR), the boot splash, startup narration, `AgentChat` "is starting", generation narration, voice phases, and spinners inside the control that started an action.
