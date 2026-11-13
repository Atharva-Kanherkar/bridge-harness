# feat-work-surface — Test Contract

Issue #976. Pull requests and background work leave the transcript. A one-line
**shelf** inside the composer frame gives the glance; a **Work** pane in the
right dock gives the detail. Plan note: `bridge://p/01a1d3cf-5739-7032-a5ca-beddaa8ad664`.

## Functional Behavior

### Shelf (inside the composer frame, above the input)

| State | Shelf line |
|---|---|
| No PRs, no background tasks | Not rendered. Composer DOM and height identical to today. |
| 1 background task, no PRs | Task glyph, the task's label, elapsed time. |
| 2+ background tasks, no PRs | "N running · M watching" (zero parts omitted), the oldest elapsed time. |
| 1 PR, checks running | Ring glyph, `#n`, truncated title, "8 of 9 · test (rust) running" (names the check when exactly one is in progress, else "8 of 9 running"; "N queued" when nothing has started). |
| 1 PR, a check failed | Red failed glyph, `#n`, title, "lint failed" (one failure) or "2 checks failed" in `text-destructive`, and a **Fix it** button. |
| 1 PR, all green | Ring glyph, `#n`, title, "Checks passed" in `text-success`. No button. |
| 1 PR, merged / closed | Merge glyph `text-info` "Merged" / muted "Closed". |
| 1 PR, no checks | "No checks" muted. |
| 2+ PRs | Stacked glyphs (worst first, at most 3), "N pull requests", then counts in this order: "x failing" (destructive), "y running", "z passed", "w merged" (info), "v closed"; zero counts omitted. |
| PRs and tasks | PR part grows; task part sits after a hairline divider, fixed width. |
| Draft PR | Phrase prefixed "Draft · ". |
| Stale PR (GitHub unreachable) | A cloud-off glyph after the phrase and "last known" in the accessible name. |

- Height is the same (36px, `h-9`) for every non-empty state, including the 4s
  finish notice.
- A task that leaves the list swaps the task part's text for "label · Done /
  Failed / Stopped" for 4000ms, in place. Only when the runtime reported an
  outcome; never for a launch acknowledgement, a lost task or another chat.
- Clicking the line opens the dock on the Work pane. With exactly one PR the
  pane opens with that PR expanded.
- **Fix it** puts a prompt in the composer naming the PR number, each failing
  check and its log URL when present, and does not send.
- No element of this feature is `sticky`, `fixed` or `absolute`, and nothing is
  rendered inside the transcript scroller.

### Work pane (dock pane id `work`, label "Work")

- First tab in the dock strip; always available (no repository needed).
- Tab badge = PR count + background task count; tab alert when any open PR has
  a failing check.
- Section "Pull requests" with "+ Attach" (the existing attach form). With no
  PRs the section still shows the attach control and a one-line empty state.
- PR rows: glyph, `#n` + title, status at right, second line branch + short
  sha. Ordered worst first: failing, running, passing/none, closed, merged;
  stable within a phase.
- One PR expanded at a time. Expanded shows checks via `orderChecks` (failed
  first) with a status icon, name, workflow, and a Logs link when there is a
  log URL; then actions: "Fix the failing check" (only when failing),
  "Open in GitHub pane" (only when a repo exists), "GitHub ↗" link.
- A stale PR shows the existing "GitHub didn't answer" notice with Retry.
- Section "In the background": one row per task (glyph, label, elapsed,
  detail line). Expanding shows the output tail or "Output goes to …" and the
  command — the same content the island showed.
- Empty pane (no PRs, no tasks): "Nothing running" copy plus the attach control.
- The status live region (`aria-live="polite"`) moves from the cards to the
  shelf and still speaks only on a phase transition.

### Removed

`ChatPullRequestCards` (trailing transcript section), `ChatPullRequestStrip`,
`useInView`, `jumpToChatPullRequests`, the 44px `StatusRing`, `CheckStrip`,
`CountLegend`, the `BackgroundActivity` island component and its mount in
`AgentConversation`.

### Data flow

- `AgentConversation` takes `onBackgroundWork?(snapshot)` and calls it with
  `{ scope, items, signature, outcomeOf }` from an effect; read-only and
  preview transcripts never call it.
- `backgroundTasks.ts` store notifies subscribers only when `scope` or
  `signature` changes; a same-signature publish replaces `outcomeOf` silently.
- `useBackgroundTasks(scope)` returns `{ tasks, notice }` for that scope only;
  its clock ticks only while tasks exist.

## Unit Tests

`src/workSummary.test.ts`
- `orderPullRequests` puts failing, running, passing, none, closed, merged in that order, stable within a phase.
- `shelfPhrase` for: running with one in-progress check (names it), running with two, queued-only, one failure (names it), two failures, passing, merged, closed, no checks, draft prefix.
- `prCounts` → the multi-PR parts in order with zero parts omitted.
- `taskSummary` for 0, 1, 2 running, 1 running + 1 watching, watching only; oldest elapsed.
- `fixPrompt` names `#n`, every failing check, and includes log URLs only when present.

`src/backgroundTasks.test.ts` (moved helper tests keep passing unchanged)
- `redirectTargets`, `formatElapsed`, `describeBackgroundTask`, `backgroundOutcome` — the existing cases.
- store: notifies on a new signature; does not notify on a same-signature publish but the latest `outcomeOf` is used; a different scope notifies.

`src/dockLayout.test.ts`
- `work` is a valid pane id; a persisted `work` dock restores to `work`.

## Integration / Functional Tests (jsdom)

`src/components/WorkShelf.test.tsx`
- Renders nothing with no PRs and no tasks.
- One line for each row of the state table above (text and tone class).
- "Fix it" appears only for a failing single PR and calls `onFix` with that PR.
- Clicking the line calls `onOpen` with the lone PR's number (or undefined for 2+).
- Finish notice: a task leaving with outcome "done" shows "Done" then clears after 4000ms; no outcome → no notice; a scope change → no notice.
- The live region text changes only on a phase transition.

`src/components/WorkPane.test.tsx`
- Empty state shows "Nothing running" and the attach control.
- PRs render worst-first; clicking a row expands it and collapses the other.
- Expanded failing PR lists the failed check first and offers "Fix the failing check".
- `focusNumber` opens with that PR expanded.
- Stale PR shows the notice and Retry calls `onRetry`.
- Task row expands to its output tail / log file names and command.

`src/components/ChatPullRequests.test.tsx`
- `useChatPullRequests` cases stay as they are.
- Card and strip cases are replaced by `PullRequestGlyph` cases (ring for open, failed glyph when failing, merge glyph when merged).

`src/components/SessionDock.test.tsx`
- A pane outside the first nine `DOCK_PANES` entries advertises no ⌥⌘ chord in its title.

`src/designSystem.test.ts` stays green (no palette classes, hex, washes, blur at rest).

## Smoke Tests

- `bunx tsc -b` clean.
- `NODE_OPTIONS=--no-experimental-webstorage bunx vitest run` all green.
- `bun run build` green.

## E2E Tests

N/A — no browser E2E suite covers the chat surface. Covered by the manual mock-mode path below.

## Manual / cURL Tests

Mock mode, `bun run dev` (or `bunx vite --port 1421 --strictPort`), open `/?mockChatPrs`:
1. Open a chat. Shelf shows `#341 … ` with its live phrase; no card in the transcript, no floating pill.
2. Click the shelf → dock opens on Work with #341 expanded.
3. In Work, Attach `#342` (even number fails in mock) → shelf becomes "2 pull requests · 1 failing · …", Work lists #342 first.
4. Open a chat with no PRs → no shelf; Work pane still reachable from the dock and offers Attach (reachable from a fresh install with UI alone).
5. Dark and light screenshots of: single running, single failing, multi-PR + tasks, Work pane expanded.

cURL: N/A — frontend only, no wire change.
