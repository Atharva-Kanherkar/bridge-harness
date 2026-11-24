# Trails scrolling and scroll motion: test contract

Locked before implementation, on branch `fix/trails-scroll`.

## Problem

`App.tsx` mounts every destination inside `<main class="overflow-hidden flex flex-col">`, and each screen brings its own scroller (`ProjectsScreen` uses `h-full overflow-y-auto`). Trails has no scroller, so a long list or timeline is clipped at the bottom of the window and can't be reached.

## Expectations

1. **The list scrolls.** `TrailsList` renders inside a `[data-trail-scroll]` element that has `overflow-y-auto`, `min-h-0` and `flex-1`. The scroller is the outermost element the component renders.
2. **A trail's page scrolls.** `TrailPage` renders inside the same scroller, in all three states: with no summary yet, with a summary while loading, and fully loaded.
3. **The skeleton matches.** `TrailsScreenSkeleton` uses the same scroller, so the Suspense fallback lays out the same way.
4. **Scroll resets per trail.** When `TrailPage` switches to a different trail id, the scroller's `scrollTop` goes back to 0.
5. **The condensed bar.** Each scroller has a sticky bar marked `[data-trail-bar]`:
   - At `scrollTop` 0 it has `data-shown="false"`, `aria-hidden="true"` and `inert`, so neither the pointer nor the keyboard can reach it.
   - Past the threshold (72 px) it has `data-shown="true"` and leaves the accessibility tree free.
   - Scrolling back under the threshold hides it again.
   - On a trail's page it shows the trail's title and a "Continue this trail" action that calls `onContinue`. On the list it shows "Trails" and a "New trail" action that opens the form.
6. **Back to the top.** Clicking the bar's title scrolls the scroller to the top. The scroll is `smooth`, or `auto` when `prefers-reduced-motion: reduce` matches.
7. **No re-render on scroll.** Scroll progress is written to the `--trail-scroll-progress` CSS variable on the scroller, a value from 0 to 1. Only crossing the threshold changes React state.
8. **Entrance motion.** List rows and timeline events carry `trail-rise` with a `--trail-rise-delay` that steps by index and stops growing after 12 rows. Timeline events also carry `trail-reveal`, a scroll-driven fade-in applied only under `@supports (animation-timeline: view())`.
9. **Reduced motion.** `src/index.css` turns off `trail-rise`, `trail-reveal`, the bar's slide and the progress hairline under `prefers-reduced-motion: reduce`.
10. **No regressions.** Every existing Trails test passes unchanged: `TrailPage.test.tsx`, `TrailPeek.test.tsx`, `TrailsScreen.test.tsx`, `App.trails.test.tsx` and `src/designSystem.test.ts`.

## Gates

- `bunx vitest run src/components/trails src/components/TrailsScreen.test.tsx src/App.trails.test.tsx src/designSystem.test.ts`
- `bun run build`
- Screenshots in dark and light of the list and a trail's page, plus one in the scrolled state with the condensed bar showing.
