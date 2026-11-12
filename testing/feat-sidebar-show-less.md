# feat/sidebar-show-less — Test Contract

A sidebar group with more than `GROUP_ROW_CAP` (12) chats shows its first 12
and a "Show N more" control. Today that control is one-way: once a group is
expanded, the only way back to the short list is changing the grouping. This
branch makes the control a toggle.

## Functional Behavior

- A group with more than 12 chats renders 12 rows and a "Show N more" button,
  where N is the hidden count. Unchanged.
- Clicking "Show N more" renders every chat in that group and replaces the
  button with "Show less" in the same place (the end of the group).
- Clicking "Show less" returns that group to 12 rows and "Show N more".
- The toggle is per group: expanding or collapsing one project leaves every
  other group as it was.
- A group with 12 or fewer chats shows neither control.
- A folded group shows neither control (folding wins, as it does today).
- The control reports its state: `aria-expanded="false"` on "Show N more",
  `aria-expanded="true"` on "Show less".
- Changing the grouping still forgets every expansion, so the next view starts
  capped.
- Styling matches the existing control: same row height, indent and muted text.

## Unit Tests

- `BridgeSidebar.test.tsx` › "caps a group and offers the rest behind one
  control" still passes (static render: "Show 3 more", 13th row absent).
- Same file: a group of exactly 12 renders no "Show" control.

## Integration / Functional Tests

`BridgeSidebar.interaction.test.tsx`, jsdom:

- "Show 3 more" expands the group, then "Show less" collapses it back: row
  "Chat 0" appears then disappears, and the button label and `aria-expanded`
  flip each time.
- With two project groups over the cap, expanding one leaves the other on
  "Show N more".
- Folding an expanded group hides "Show less"; unfolding brings it back.

## Smoke Tests

- `bun run build` green.
- `bun run test` green for the frontend suites (Rust and sidecar untouched).

## E2E Tests

N/A. No E2E harness covers the sidebar; the jsdom interaction suite is the
closest layer.

## Manual / cURL Tests

- `bun run dev`, group by Project, open a project with more than 12 chats (or
  seed the mock with them): click "Show N more", confirm all rows show and the
  last row is "Show less"; click it, confirm the list returns to 12.
- Repeat in light and dark themes for the PR screenshots.
- No cURL surface: this is a frontend-only change.
