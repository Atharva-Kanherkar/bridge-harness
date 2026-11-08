# feat/notes-phase-3-views: Test Contract

Issue #890 (parent #886, epic #865). Builds on #888 (`notes/query_database`, `save_view`,
`delete_view`, the filter grammar) and #889 (`DatabaseView`, `DatabaseTable`, `PropertyCell`,
`formatValue`, `schemaWrites`). No wire or protocol change.

## Functional Behavior

### View model (`src/components/notes/database/viewModel.ts`, pure)
- `opsFor(column)` lists the ops a property takes, exactly as #888's planner does:
  - text, url, `title`, `createdBy`: contains, notContains, equals, notEquals, startsWith, isEmpty,
    isNotEmpty.
  - number: eq, ne, lt, lte, gt, gte, isEmpty, isNotEmpty.
  - date, `created`, `updated`: before, after, onOrBefore, onOrAfter, equals, isEmpty, isNotEmpty,
    within.
  - select, `kind`, `state`, `project`: is, isNot, isAnyOf, isNoneOf, isEmpty, isNotEmpty. Status
    adds inGroup.
  - multiSelect, relation, `tags`: contains, notContains, containsAny, isEmpty, isNotEmpty.
  - checkbox: is. `chats`: isEmpty, isNotEmpty. `parent`: isEmpty, isNotEmpty, equals, notEquals.
- `OP_LABELS` gives each op its words ("is", "is not", "contains", "≥", "is within", …).
- `needsValue(op)` is false for isEmpty and isNotEmpty.
- `defaultCondition(column)` is the column's first op with an empty value of the right shape:
  - `""` for text and dates.
  - `0` for numbers.
  - `false` for checkbox.
  - The first option id for select, status and `kind`/`state`.
  - `[]` for list ops, and `"pastWeek"` for `within`.
- `retarget(condition, op)` keeps a value that fits the new op, and otherwise resets it:
  - A scalar becomes `[scalar]` for isAnyOf, isNoneOf and containsAny, and back.
  - `value` is dropped for isEmpty and isNotEmpty.
- `filterTree` helpers edit #888's grammar with paths: `addCondition`, `addGroup` (one level deep,
  so a group inside the top group, never deeper), `setCondition`, `removeAt`, `setJoin` (and/or).
  - The top level is always a group.
  - A group that empties is removed.
  - The tree never holds more than 20 conditions. `canAdd` is false at 20.
- `groupableColumns(database)`: select, status, multiSelect, checkbox, date and the system `kind`,
  `state` and `project`. `sortableColumns(database)` leaves out multiSelect, relation, `tags`,
  `chats` and `parent`.

### Unsaved overrides (`useViewOverride`)
- A view's filter, sorts and grouping edited from the toolbar apply at once as an override: the
  query sends `filter`, `sorts` and `groupBy` explicitly beside `viewId`.
- While an override differs from the saved view, the toolbar shows "Save for everyone" and
  "Reset":
  - "Save for everyone" calls `notes/save_view` with every other field kept, then drops the
    override.
  - "Reset" drops the override and the query goes back to the saved view.
- The override lives in component state, keyed by view id, and switching tabs keeps each tab's.

### Toolbar (`ViewToolbar.tsx`, `FilterBuilder.tsx`)
- Filter:
  - A builder of rows: property, then op (`opsFor`), then value.
  - The value editor fits the op:
    - a text or number input;
    - a date input, or the `within` choice list;
    - option pickers for select, status and multiSelect (multiple for isAnyOf, isNoneOf and
      containsAny);
    - Yes/No for checkbox;
    - a group picker for inGroup;
    - nothing for isEmpty and isNotEmpty.
  - "Add a filter" and "Add a filter group" (one level deep), an and/or switch per group, and a
    remove button per row.
  - A number field that is not a finite number leaves the condition out of the override until
    fixed.
- Sort: up to five rows of property and direction, reorderable with up and down buttons, plus
  "Add a sort" while there are fewer than five.
- Group: the `groupableColumns`, plus a date bucket (day, week, month) for dates, plus "None":
  - Board requires a grouping, so it has no "None".
  - Calendar has no Group control.
- The filter button shows the condition count, and the sort button the sort count.

### Views (`ViewTabs.tsx`)
- Tabs in `order`, each showing its type's icon.
- "+ New view":
  - Choose table, board, calendar or list, then type a name.
  - It saves with `order` after the last view.
  - A board gets `groupBy` on the first groupable property.
  - A calendar gets `dateProperty` on the first date property, or `created`.
- Each tab's menu:
  - Rename and Change type, which save that field.
  - Duplicate, which saves a copy named "<name> copy" right after the original.
  - Delete, which is disabled on the last view.
- Reorder:
  - Drag a tab, or use the menu's Move left and Move right.
  - The moved views' `order` is saved, renumbered 0..n-1.

### Board (`DatabaseBoard.tsx`)
- Columns come from the query's `groups`, in order, each with its label and count.
- Each column shows its first `groupLimit` (50) cards. Cards show the title and the visible
  properties' `formatValue` text.
- "Load more" in a column re-queries with that column's condition added to the filter, and appends
  the next page by cursor.
- Moving a card to another column writes the grouped value with `notes/update_note`:
  - select or status: the target option id, or `null` for "No value".
  - multiSelect: the source option is swapped for the target. To "No value" removes the source
    option.
  - checkbox: `true` or `false`.
  - `kind` and `state`: those fields.
  - `project`: `workspaceId` or `clearWorkspace`.
- The move shows at once, rolls back on error, and reports through `onError`.
- Drag and drop moves a card with the pointer. The keyboard path: a card's "Move to…" menu lists
  the other columns.
- "+ New" in a column creates a row, then sets that column's value on it.
- A board with no grouping, or whose grouping no longer runs, shows "Choose a property to group
  by".

### Calendar (`DatabaseCalendar.tsx`)
- A month grid, six weeks, with the week starting on Monday (ISO, as the planner's `thisWeek`). The
  `dateProperty` places rows:
  - `{start}` sits on its day.
  - `{start, end}` sits on every day from start to end.
  - A datetime sits on its UTC day, as the planner reads it.
- Prev, Next and Today move the month. Each month queries the view with the condition
  `{and: [<view filter>, {dateProperty onOrAfter firstShownDay}, {dateProperty onOrBefore lastShownDay}]}`.
- Dropping an item on a day writes `{start: newDay}`:
  - A range keeps its length.
  - A datetime keeps its time of day and offset.
- "No date" is a tray of rows with an empty date, queried with `isEmpty`. Its items drag onto days.
- With no date property, the calendar shows "Add a date property to use a calendar".

### List (`DatabaseList.tsx`)
- One line per row: the title, then the visible properties as compact text. Clicking a line opens
  the row.
- With a grouping, rows sit in sections under the group label and count. A section collapses with
  its header button.
- Without a grouping, the rows are windowed as in the table (36 px rows, overscan 10).

### Notes list tabs (`NotesScreen.tsx`)
- The tabs are the system database's views (`notes/read_page(systemDatabaseId)`), in order. Each
  tab's rows come from `notes/query_database` with that `viewId` and the search text as `query`,
  paging by cursor.
- `seededKind(view)` names a view whose filter equals a seeded one (all, inbox, pages, tasks, ideas,
  agent, archived). It drives the behaviors that used to hang off the five `FILTERS` ids:
  - The page tree for all and pages.
  - Pending captures on all and inbox.
  - Each empty-state sentence.
  - An edited or new view is flat, with the generic empty state.
- Capture, search, the page tree, the job polling and the open page are unchanged. Every existing
  `NotesScreen.test.tsx` case passes, with its tab labels unchanged.
- A tab's menu has three items:
  - **Edit view** opens the same Filter, Sort and Group builders in place, with "Save for
    everyone" and "Reset".
  - **Rename.**
  - **Hide**, which deletes the view and is disabled on the last one.
- "Reset to defaults" (the list's settings menu) saves the seven seeded views as new views, then
  deletes every other view, so the defaults come back exactly.

## Unit Tests
- `viewModel.test.ts`:
  - `lists each type's ops as the planner does`
  - `gives every column a default condition of the right shape`
  - `retargets a value when the op changes`
  - `edits the filter tree by path, one group deep, at most 20 conditions`
  - `lists groupable and sortable columns`
- `useViewOverride.test.tsx`:
  - `applies an override at once and saves it with every other field kept`
  - `reset drops the override`
  - `keeps each tab's override apart`

## Integration / Functional Tests
- `FilterBuilder.test.tsx`:
  - `builds each op's JSON from the rows`
  - `adds a group one level deep and switches and/or`
  - `leaves an unfinished number out`
- `ViewToolbar.test.tsx`:
  - `sorts reorder and stop at five`
  - `board has no None grouping and calendar no group`
- `ViewTabs.test.tsx`:
  - `new view of each type saves its defaults`
  - `rename, duplicate, change type and delete save, and the last view cannot be deleted`
  - `moving a tab saves the order`
- `DatabaseBoard.test.tsx`:
  - `shows one column per group with its count`
  - `dragging a card writes the grouped value and rolls back on error`
  - `Move to… moves a card from the keyboard`
  - `a multi-select move swaps the option`
  - `Load more pages one column`
  - `New in a column makes a row with that value`
- `DatabaseCalendar.test.tsx`:
  - `places single dates, ranges and datetimes on their days`
  - `queries each shown month and moves with prev, next and today`
  - `dropping on a day keeps time and range length`
  - `the No date tray holds undated rows and drags onto a day`
  - `asks for a date property when there is none`
- `DatabaseList.test.tsx`:
  - `shows rows with their properties and opens one`
  - `groups into collapsible sections`
- `DatabaseView.test.tsx`:
  - `routes each view type`
  - `a nested or filter, two sorts and a grouping apply at once and save for everyone`
- `NotesScreen.test.tsx`:
  - The existing cases, unchanged.
  - `the tabs are the system views and show the same lists as before`
  - `editing the Tasks tab narrows it, and Reset to defaults restores it`
  - `hide removes a tab, but never the last`
- `notesMock.test.ts`:
  - `the seeded system views return the old FILTERS lists`

## Smoke Tests
- `bun run build` and `bun run test` are green.
- In `bun run dev`, the Reading list opens as a table, a board by Status, a calendar by Due and a
  list. Each toolbar control works, and the Notes tabs are the seven views.

## E2E Tests
- N/A as automated tests. The acceptance flow runs by hand in `bun run dev`, and the screenshots go
  in the PR.

## Manual / cURL Tests
- Screenshots in the PR body, dark and light:
  - the board;
  - the calendar;
  - the list;
  - the filter builder;
  - the edited Notes tabs.
- 1000 rows: a board column renders its first 50 cards and pages with "Load more". An ungrouped list
  is windowed.

## Amendments after review (PR #903)

Each item below came out of review and has a regression test that fails without its fix.

- **Composed filters stay within the planner's limits.** A body that adds conditions to the view's
  filter (Calendar's window, a board column's next page) composes them so the result still nests at
  most two levels and holds at most 20 conditions:
  - A top `and` takes the new conditions as items.
  - A top `or` distributes them into each branch.
  - When that would break a limit, the body reads the view's own filter, follows every cursor, and
    applies the extra conditions to the rows itself.
- **A date column pages by its interval.** A board grouped by a date bucket loads more of a column
  with `onOrAfter`/`onOrBefore` around the bucket: the day, the week from its Monday, or the month.
  "No value" uses `isEmpty`.
- **The calendar finds ranges that overlap it.** The core indexes a date's start only, so the month
  reads rows whose start is on or before the last shown day. It keeps those whose end, or start,
  is on or after the first shown day.
- **Calendar reads follow their cursors.** Both the month and the "No date" tray read every page.
- **Board pages load once per column.** Load more is disabled while its column's request is out, and
  appended rows are deduplicated by id.
- **A board page from an earlier query is dropped.** Each server answer starts a new generation, and
  a column page that belongs to an older one is ignored.
- **A failed save keeps the override.** Saving reports real completion. On a refusal, the edit and
  "Save for everyone" stay, ready to retry. This holds in `DatabaseView` and on the Notes tabs.
- **Saving clears only what was saved.** An edit made while a save is out stays as an unsaved
  override.
- **Reorders queue with every other view edit.** Moving a view goes through the same ordered,
  composed save as renames and filter changes, so a slow reorder never writes an older name back.
- **A number filter shows what it applies.** The number field follows its condition's value when the
  row changes under it (a row above removed, the property switched).
- **A Project filter starts on a real project.** Its first value is the first workspace. When the
  value matches no choice, the picker shows "Choose…".
- **The Notes list keeps its newest snapshot.** A list read that lands after a newer one never
  replaces the page list used for links, breadcrumbs and child pages.
- **An empty list still measures its scroller.** The list's scroller stays mounted when it has no
  rows, so rows that arrive later are windowed against the real height.

### Second review round (PR #903)

- **The fallback reads page fields too.**
  - The calendar's client-side window reads `created` and `updated` from the page.
  - The board's client-side grouping reads `kind` and `state` from the page, as the planner does.
- **An empty group stays true when composing.** A filter whose top `or` holds an empty group matches
  every row, so composing into it treats the base as no filter and adds only the new conditions.
- **Filter rows keep their identity.** Each row keeps a stable key through edits, so unfinished text
  (`4x`) stays with its own condition when a row above it is removed.
