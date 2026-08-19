# feat/work-facts-ui — Test Contract

Issue #206, slice 3 of 7 under epic #203. The first independently shippable
increment. Locked before implementation; design approved in
[`docs/work-view-design.html`](../docs/work-view-design.html).

Slices 1 and 2 are merged: `work/get_work_board` serves a deterministic board from
SQLite, and briefing authority exists but has no runner. This slice renders the
facts half and nothing else.

## The shape of the problem

Work is already the name of the rail's scope pill — the half that is not Code, where
plain conversations live. The board belongs there rather than in the footer nav:
Code's right side is a conversation, so Work's right side should be what needs you.
Flipping the pill is the gesture that opens it, and a chat is one click below.

The board must be worth opening on a plane. Everything it shows was already recorded
by Bridge, so opening it starts nothing.

## Functional Behavior

### Navigation

- Work is an explicit app view (`view === "work"`), not a variant of selected-session
  state. Selecting a session leaves it.
- Flipping the scope pill to Work opens the board. Flipping to Code leaves it and
  restores the conversation surface.
- A `Needs you` row sits above the chat list when the scope is Work, carries
  `aria-current="page"` while the board is open, and returns to it from a chat.
- The row shows a count of blocking + attention facts, and **no badge at all** when
  that count is zero — never a zero.
- The window title strip reads `Work` while the board is open.

### Opening it starts nothing

- Rendering the board calls exactly one thing: `api.workBoard()`. No session is
  selected or created, no model, connector, git command, or network request starts.
- Asserted by a spy over the whole api surface: on mount, `workBoard` is the only
  member called.

### Facts

- Rendered in the order the backend returned. The client never re-sorts.
- Grouped under band headings by severity — the primary sort key, so grouping costs
  nothing and reorders nothing. A band with no facts is absent, not empty.
- Each row carries: a 3px severity edge, a monochrome source tile, the severity and
  source as text, a title, a detail, context chips, freshness, and one action.
- Severity, source and freshness are words in the accessible name, never colour
  alone.
- Every fact kind maps to its own source glyph: check, approval, queue, branch.
- Each of the four `WorkFactAction` variants invokes its own api call and reports
  success, failure, and retry.

### Freshness

Three tells, so no single one carries it alone:

- **Tense** — `live` says "has drifted", `stale` says "had drifted".
- **Dimmed numbers** — a stale detail renders at reduced opacity; `unknown` renders
  no numbers at all.
- **The action changes** — `live` offers the fast-forward, `stale` and `unknown`
  offer to measure again. A number nobody has checked recently is not one to act on,
  so the fast-forward is absent rather than disabled.

`stale` and `unknown` are each visually and textually distinct from `live`.

### States

- **Loading** — three skeleton rows in the shape of the answer. No spinner.
- **Empty** — "Nothing needs you", stated plainly. No illustration, no celebration.
- **Board failed to read** — explains that the screen only reads, so retrying is
  safe, and offers Try again.
- **Action failed** — the fact stays. The failure attaches to its row with the real
  reason from the backend, and the action becomes Try again.
- **No briefing model / no connector** — one quiet dismissible line saying Suggested
  work is off and the facts below do not need it. Not an empty section with a call
  to action.
- **Offline** is not a distinct state: the board never needed the network.

### Styling

- Tailwind v4 utilities and existing tokens only. No new stylesheet, no CSS module,
  no v3 config, no inline static style attributes.
- `src/designSystem.test.ts` stays green, which means no raw palette class, no
  `white/10`-style alpha, no hex literal, and no blur on a resting surface.

## Unit Tests

`src/components/workFacts.ts` — presentation logic, no React:

- `bandsFollowTheBackendOrder` — facts are banded by severity without reordering
  within a band.
- `anEmptyBandIsAbsent`
- `severityAndFreshnessAreWordsInTheAccessibleName`
- `aStaleDivergenceReadsInThePastTense`
- `anUnknownDivergenceClaimsNoNumbers`
- `theActionLabelFollowsFreshness` — live → fast-forward, stale/unknown → measure
  again.
- `everyFactKindHasItsOwnSourceGlyph`
- `everyActionVariantHasALabel` — exhaustive over the four variants, so a fifth
  fails to compile.
- `relativeTimeReadsInWholeUnits` — "just now", "4 min ago", "2 h ago", "3 d ago".
- `anUnparseableObservedAtDoesNotRenderAsInvalidDate`
- `theCountExcludesInfo` — the rail badge counts blocking and attention only.

`src/components/WorkView.test.tsx` — the board:

- `everyFactKindRendersItsRowAndAction`
- `factsRenderInTheOrderGiven`
- `loadingShowsSkeletonRowsNotASpinner`
- `anEmptyBoardSaysNothingNeedsYou`
- `aFailedReadOffersARetryAndSaysReadingIsSafe`
- `aFailedActionKeepsTheFactAndShowsTheRealReason`
- `aStaleFactOffersMeasureAgainAndNotAFastForward`
- `anUnknownFactOffersMeasureAgain`
- `theSuggestedWorkNoticeIsQuietAndDismissible`
- `openingTheBoardCallsOnlyWorkBoard`
- `aLongUntrustedLabelWrapsRatherThanOverflowing`
- `eachRowIsOneTabStopThenItsAction`
- `theListHasAnAccessibleName`

`src/components/BridgeSidebar` additions:

- `flippingThePillToWorkOpensTheBoard`
- `flippingToCodeLeavesTheBoard`
- `theNeedsYouRowIsCurrentWhileTheBoardIsOpen`
- `theNeedsYouRowShowsNoBadgeWhenNothingNeedsYou`
- `selectingAChatLeavesTheBoard`
- `theNeedsYouRowIsAbsentInCodeScope`

## Integration / Functional Tests

`src/App.test.tsx` additions:

- `theWorkViewIsLazyLoadedBehindSuspense`
- `openingWorkDoesNotSelectOrCreateASession`
- `existingSessionSelectionAndMissionControlAreUnchanged` — the regression that
  matters: the conversation, Mission Control, Marketplace, Settings and Projects
  paths behave exactly as before.
- `theTitleStripReadsWorkWhileTheBoardIsOpen`

`src/api.test.ts` additions:

- `workBoardCallsTheGeneratedMethod`
- `theBrowserFallbackReturnsAUsableBoard` — so vitest and a browser preview render
  something real without Tauri.

## Smoke Tests

- `npx vitest run` green.
- `bun run build` green.
- `cargo test --manifest-path src-tauri/Cargo.toml --workspace` green — this slice
  touches no Rust, so this is a regression check.
- `npm test --prefix sidecar/claude-agent` green.
- Regenerating protocol artifacts leaves the tree clean; no wire method is added.

## E2E Tests

N/A — the repo has no browser-driven E2E harness. The component tests drive real
DOM through `react-dom/client` and `act`, which is the closest equivalent and is
what every other screen in this repo is tested with.

## Manual / cURL Tests

No HTTP surface. Manual verification is the app itself:

```bash
npx vitest run src/components/WorkView.test.tsx src/components/workFacts.test.ts
```

Then in the running app: flip the rail pill to Work, confirm the board is what
appears on the right, confirm the `Needs you` count matches the row count excluding
info, open a chat and return via the row, and flip to Code to confirm the
conversation comes back.
