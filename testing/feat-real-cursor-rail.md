# feat/real-cursor-rail — Test Contract

Restyle `BridgeSidebar` to match the Cursor mock's chrome, action rows, compact
grouped list, and dense footer, without replacing live data or the routes the
rail already owns.

This contract **supersedes visual rules** in `testing/feat-sidebar-redesign.md`
and `testing/feat-projects-screen-compact-header.md` where they conflict (row
height, New-chat treatment, search control placement, footer contents). It does
**not** rewrite those files' data rules: scope, hidden session kinds, filter
persistence, `GROUP_ROW_CAP = 12`, Needs-you, Memory.

Plan (branching, locked decisions): `docs/plans/real-sidebar-from-mock.md`.
Epic: #309. PRs target `feat/cursor-sidebar-dev`, never `main`.

## Scope And Stated Assumptions

- **The rail is the only surface that changes**, plus the tiny amount of
  `App.tsx` / `AppTitleBar` / `SessionToolbar` needed to host window chrome.
- **`CursorSidebarMock.tsx` is a look-at file until Phase 6.** The running app
  does not render it. Phase 6 deletes it.
- **No protocol/backend change.** Git presence is `Workspace.branch`; there is
  no cloud-sync field; do not invent one.
- **No fake account.** The mock's avatar + "Yashaswi Kumar" has no real object
  behind it and does not ship.
- **Customize opens Settings**, not Projects. Projects stay a footer row
  (and a new-folder control on the list header).
- **Default `groupBy` stays `"date"`.** Code-scope users who want repo groups
  pick `Group by → Project` in the existing filter menu.
- **`chatListTime` is new and lives in `sidebarChats.ts`.** Compact (`2h`,
  `4m`, `1d`), not `workDashboard.relativeTime` (`2h ago`).

## Functional Behavior

### Chrome strip (Phase 2)

- Expanded rail, `showWindowNav`: an `h-11` strip with `pl-24` (traffic-light
  inset) and `data-tauri-drag-region="deep"`. Panel toggle on the left of that
  strip, back/forward chevrons pushed right (`ml-auto`).
- Collapsed rail, `showWindowNav`: panel-only, centered, no `pl-24`.
- `showWindowNav={false}` (fullscreen): the rail does not render panel or
  chevrons. `AppTitleBar` receives `leading={WindowPanelButton}` and
  `trailingNav={WindowHistoryChevrons}`.
- Chevrons are `disabled` when `canBack` / `canForward` is false.
- Back/forward walk `navigationHistory.recordPlace` in `App.tsx` (view +
  session id). A no-op when the place has not changed. Going back trims
  forward from the previous tip.
- Session toolbar does not also apply `pl-24` in fullscreen.

### Action rows (Phase 3)

Under the chrome strip, expanded, in this order, each an `h-7` ghost row
with a 15px icon and 13px label:

| Label | Handler |
| --- | --- |
| New Chat | `onOpenNewChat` |
| Search | toggles the existing filter input; closing clears the query |
| Automations | `onOpenMarketplace` |
| Customize | `onOpenSettings` |

- Search no longer lives on the Chats/Repositories header. That header keeps
  `SidebarFilterMenu`.
- Collapsed: the four rows become icon-only 9×9 buttons with `title` and
  `aria-label` matching the labels above.
- The filled primary `New chat` button is gone.

### Scope switch (Phase 3, surviving)

- Sits **below** the action rows and **above** Needs-you.
- Work / Code tabs, persisted `bridge.sidebar.scope`, default Work.
- Opening a chat still follows that chat's scope once per id.
- Work with a persisted `groupBy: "project"` still corrects to `"date"`.
- Collapsed: icon-only tabs, still reachable.

### Needs-you (Phase 3, surviving)

- Work scope only. Hidden in Code.
- Between the scope switch and the list.
- Count badge only when `workNeedsYouCount > 0`.
- `aria-current="page"` while `workBoardActive`.
- Click calls `onOpenWorkBoard`. Flipping the scope switch to Work still
  opens the board (existing behavior).
- Collapsed: icon-only with `title="Needs you"`.

### List header (Phase 4)

- Work: label `Chats`. Code: label `Repositories`.
- Right side: `SidebarFilterMenu` and a new-folder control that calls
  `onOpenProjects`.
- Filter menu behavior unchanged (status/agent/group/sort, persistence,
  project grouping offered in Code only).

### Chat rows (Phase 4)

- Height 26px expanded (`h-[26px]`), `pl-7` when a group label is shown.
- Visible: status dot, truncated `chatName`, optional git icon, compact time.
- Git icon (`aria-label` includes "git" or "branch") only when
  `workspaces.find(w => w.id === chat.workspaceId)?.branch` is a non-empty
  string. A workspace with `branch: null` / `""` / missing has no icon.
- No cloud icon.
- No harness or model in visible text. `title` attribute remains
  `<name> — <Harness> · <model>` when a model is set.
- Time from `chatListTime(chatTimestamp(chat), now)`:
  - invalid / missing timestamp → not rendered (no "unknown" string on the row)
  - `< 60s` → `now`
  - `< 60m` → `Nm`
  - `< 24h` → `Nh`
  - else → `Nd`
- Active row: `bg-accent` + medium weight. Status tokens unchanged.

### Groups, cap, fold (Phase 4)

- `GROUP_ROW_CAP` stays **12**. Copy: `Show N more`.
- Search (non-empty trimmed query) bypasses the cap, as today.
- Collapsed rail does not cap and does not fold.
- Fold still hides rows and keeps the header + count.
- Default grouping remains Date. Project grouping still labels workspace
  titles and a trailing `No project` group. The no-project group uses a Home
  icon; named project groups use a Folder icon.

### Footer (Phase 5)

- Expanded: **Projects**, **Memory** only. No Marketplace row, no Settings
  row, no avatar, no person name.
- Active styling on Projects when `projectsActive`.
- Collapsed: the same two as icon-only, plus the Phase 3 action icons.
- Memory still reachable with zero workspaces.

### Mock deletion (Phase 6)

- `src/components/CursorSidebarMock.tsx` is gone.
- `COLOR_LITERAL_ALLOWLIST` does not mention it.
- `App.tsx` contains neither `SHOW_CURSOR_SIDEBAR_MOCK` nor `CursorSidebarMock`.

### Unchanged (every phase)

- Collapse / persisted width / drag-to-resize / below-`sm` drawer + scrim.
- `visibleChats` / hidden briefing, suggestion, extraction kinds.
- `data-tauri-drag-region` on the chrome strip.
- Semantic status tokens; `src/designSystem.test.ts` stays green.
- No new `.css` file, no `tailwind.config.js`, no CSS-in-JS.

## Unit Tests

`src/navigationHistory.test.ts` — Phase 2, new:

- `placesEqual` is view + session id.
- `recordPlace` no-ops when the next place equals the current one.
- `recordPlace` appends and advances the index.
- `recordPlace` from a mid-stack index drops the tail (no divergent future).

`src/components/WindowNavButtons.test.tsx` — Phase 2, new:

- Panel click toggles collapse; chevrons fire back/forward.
- `spread` puts panel before chevrons and wraps chevrons in `ml-auto`.
- Disabled chevrons when `canBack`/`canForward` are false.
- Collapsed panel label is "Show sidebar".

`src/components/sidebarChats.test.ts` — Phase 4, extend:

- `chatListTime` — `now`, `4m`, `2h`, `3d` at the bucket edges.
- `chatListTime` — invalid input returns `null` (so the row can omit it).

Existing `dayLabel` / `groupChats` / `filterChats` / `readChatView` cases stay.

`src/components/BridgeSidebar.test.tsx`:

Phase 2:

- Expanded + `showWindowNav`: `pl-24`, "Hide sidebar", `aria-label="Back"`,
  `ml-auto`.
- `showWindowNav={false}`: none of those window controls.

Phase 3:

- Visible text includes `New Chat`, `Search`, `Automations`, `Customize`.
- Does not include a filled primary "New chat" as the only new-chat control
  (the old `bg-primary` new-chat button is gone).
- Needs-you still in the Work markup; absent in Code.
- Search field absent until the Search row is used (static markup: the closed
  state has no "Filter chats" input). The header no longer contains
  `aria-label="Search chats"` as a header icon — that label moves to the
  action row.

Phase 4:

- Existing day-header, cap-12, collapsed-no-cap, harness-in-tooltip cases stay.
- A Code + `groupBy: "project"` row is indented (`pl-7`).
- Git icon present for a project chat whose workspace has `branch: "main"`;
  absent when `branch` is null.
- No `aria-label` of "Synced" / cloud.
- A dated chat renders a compact time (`\d+[mhd]` or `now`).

Phase 5:

- Footer contains Projects and Memory.
- Footer does not contain Marketplace or Settings as row labels (those live
  on Automations / Customize).
- No hardcoded person name.

Phase 6 is asserted in `App.test.tsx`, not here.

`src/components/BridgeSidebar.interaction.test.tsx`:

- Existing fold / scope / follow / Needs-you cases stay.
- Phase 3: clicking New Chat / Automations / Customize fires the matching
  prop. Search toggles the filter input; Escape closes it.
- Phase 5: clicking Memory still fires `onOpenMemory`.

`src/components/AppTitleBar.test.tsx` — Phase 2, extend:

- Renders `leading` before the title and `trailingNav` after it.

`src/App.test.tsx`:

- Phase 2–5: if `SHOW_CURSOR_SIDEBAR_MOCK` is still a named constant, it is
  `false`. Prefer asserting the source contains `BridgeSidebar` and does not
  *render* `CursorSidebarMock` (no JSX `<CursorSidebarMock`).
- Phase 6: source contains neither `SHOW_CURSOR_SIDEBAR_MOCK` nor
  `CursorSidebarMock`.

`src/designSystem.test.ts`:

- Phase 2–5: `components/CursorSidebarMock.tsx` remains on
  `COLOR_LITERAL_ALLOWLIST` while the file exists.
- Phase 6: that entry is gone; the suite stays green.

## Integration / Functional Tests

- `App.tsx` still compiles against `BridgeSidebarProps` after the new chrome
  props (`showWindowNav`, `canBack`, `canForward`, `onBack`, `onForward`,
  `collapsed`, `onCollapsedChange`).
- `bunx tsc -b --pretty false` at the end of every phase.

## Smoke Tests

Per phase, the Verify block in `docs/plans/real-sidebar-from-mock.md`.
Phase 6 additionally:

```
rg -n "CursorSidebarMock|SHOW_CURSOR_SIDEBAR_MOCK" src/ && echo FAIL || echo clean
```

## E2E Tests

N/A — no browser E2E harness for the Tauri shell. Visual result is checked
against `src/components/CursorSidebarMock.tsx` until Phase 6 deletes it, then
against the Phase 7 manual pass.

## Manual Tests

Phase 7 on `feat/cursor-sidebar-dev`, Tauri and Vite, light and dark:

1. Traffic lights and panel sit on one line; chevrons sit on the right of
   that strip; empty strip space drags the window (Tauri only).
2. Collapse: 68px rail, panel centered, action icons reachable, no
   `pl-24` crowding the lights.
3. New Chat opens the real dialog. Search filters live chats. Automations
   opens Marketplace. Customize opens Settings.
4. Needs-you opens the Work board; badge hides at zero.
5. Code + Group by Project: folder groups, git badge on branched
   workspaces, compact times, Show N more at 12.
6. Footer: Projects and Memory only.
7. Fullscreen `⌥⌘F`: full-width title bar, panel left, chevrons right.
8. Wallpaper tint still washes the rail in Tauri; Vite stays opaque.
