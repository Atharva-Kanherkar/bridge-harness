# feat/conversation-skeleton-loader — Test Contract

Opening an existing chat whose history is still in flight shows a
transcript-shaped skeleton instead of a centered spinner with
"Opening conversation".

## Functional Behavior
- `AgentConversation` with a `session`, no `preview`, and
  `forestEntries === undefined` (history not fetched yet) and nothing live to
  draw renders the skeleton. Gate conditions are unchanged.
- The skeleton is shaped like a transcript: user-bubble placeholders on the
  right, prose-line placeholders on the left, one tool-row placeholder. Rows are
  marked `data-skeleton-row="user" | "prose" | "tool"`.
- It sits in the same column as the real transcript (`max-w-conversation`,
  `gap-5`) and uses the same padding per `density` (compact tiles use the
  compact padding), so content replaces it without a jump in width or indent.
- Each bar carries a slow highlight sweep (`skeleton-shimmer`), staggered per
  row so it reads top to bottom. No `animate-pulse`, no `animate-spin`.
- The whole skeleton fades in after a short delay, so a fast load never
  flashes placeholders.
- Accessibility: `role="status"`, `aria-label="Loading conversation"`, a
  `sr-only` "Loading conversation"; placeholder bars are `aria-hidden`.
- No visible copy: "Opening conversation" is gone.
- No inline `style` attribute (Tailwind-only rule).
- Reduced motion: the global `prefers-reduced-motion` rule stops the sweep and
  the fade lands on full opacity, leaving static bars.
- Unchanged: `forestEntries={[]}` shows the greeting; live content during a
  refetch shows instead of the skeleton; the skeleton drops once history
  arrives.

## Unit Tests
- `AgentConversation.test.tsx` › "shows a skeleton while an existing chat's
  history loads, not the new-chat greeting": status label present, user, prose
  and tool rows present, shimmer class present, no spinner, no
  "Opening conversation", no `animate-pulse`, no `style="`, no greeting.
- `AgentConversation.test.tsx` › "lays the skeleton out with the compact
  transcript padding in a tile": `density="compact"` uses `px-3` padding and
  not `sm:px-8`.
- Existing: "keeps the greeting for a loaded-but-empty chat", "shows live
  content through a forest refetch instead of the skeleton" stay green.

## Integration / Functional Tests
- `AgentConversation.history.test.tsx` (jsdom): skeleton present while history
  is in flight and gone once it arrives; stays green unchanged.

## Smoke Tests
- `bun run build` and `bun run test` green.

## E2E Tests
N/A: no E2E harness covers chat opening. Covered by the manual check below.

## Manual / cURL Tests
- `bun run dev`, open http://127.0.0.1:1420, select an existing chat in mock
  mode with history loading delayed; confirm the skeleton in dark and light
  themes, then that the transcript replaces it in place. Screenshot both
  themes for the PR.
- macOS Reduce Motion on: bars are static.
