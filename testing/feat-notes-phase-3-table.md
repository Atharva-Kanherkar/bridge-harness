# feat/notes-phase-3-table: Test Contract

Issue #889 (parent #886, epic #865). Builds on the database backend from #887 (merged in #892) and
#888 (merged in #895): `notes/create_database`, `update_schema`, `save_view`, `delete_view`,
`query_database`, `values` on `notes/update_note`, and `NotePage.database`, `databaseId` and `values`.
No wire or protocol change. Board, calendar and list views, the filter, sort and group builders and
saved view CRUD belong to #890.

## Functional Behavior

### Values and their display (`src/components/notes/database/formatValue.ts`, pure)
- `formatValue(column, value, context)` returns the text a cell shows. An empty cell is `""`.
  - text and url: the string as stored.
  - number: `Intl.NumberFormat` with up to 6 fraction digits, so `2.5` is `"2.5"` and `1000` is `"1,000"`.
  - checkbox: `"Yes"` or `"No"`. A missing cell reads `"No"`, as the planner treats it.
  - select and status: the option's name. An option id the schema no longer has reads `""`.
  - multiSelect: the option names in the stored order, joined by `", "`. Unknown ids are skipped.
  - date: `{start}` reads `"Nov 12, 2026"`. A datetime (a start with a time) adds `", 3:30 PM"` in the
    device's time zone. `{start, end}` reads `"<start> → <end>"`.
  - relation: the related rows' titles, joined by `", "`, or `"Untitled"` for a row with no title.
    An id the context cannot name reads `"1 page"`-style counts instead.
  - project (system): the workspace's title, or `""`. chats (system): `"2 chats"`. created and
    updated (system): the date as above.
- `columnsOf(database, view)` returns the table's columns in order:
  - `title` first, always visible.
  - Then the view's `properties` entries in their order, then every schema property the view does
    not list, in schema order. A column is visible unless its entry says `visible: false`.
  - A view entry naming a property that no longer exists is skipped.
  - System columns (`project`, `chats`, `created`, `updated`) appear only when the view lists them.
  - Widths come from the view entry, else a default by type: title 280, text and url 200, number 120,
    checkbox 96, date 160, select, status, multiSelect and relation 180, and system columns 160.
    Widths clamp to 80…640.
- `retypeNote(from, to)` names what a retype keeps, per #887's conversion rules:
  - number→text, checkbox→text, select→text and status→text keep every value.
  - text→number keeps values that read as numbers. text→checkbox keeps `true` and `false`.
  - text→select and text→status keep values and add an option per distinct value.
  - select→multiSelect keeps every value.
  - Every other pair clears all values.
  - The same type keeps every value.
- `nextWidth(start, delta)` clamps a dragged width to 80…640.

### Cell editors (`PropertyCell.tsx`)
- One component per type, used by the table and the row page's property panel. It commits the
  stored value shape. `null` means clear, and a commit never sends an unchanged value.
  - text: an inline input. Enter commits the trimmed-right text, or `null` when empty.
  - number: an inline numeric input. A value that is not a finite number is refused: the input
    shows `aria-invalid`, Enter does nothing, and Escape cancels. Empty commits `null`.
  - select and status: an option picker with a search box. Typing a name that matches no option
    offers "Create <name>", which calls `notes/update_schema` `setOptions` with the existing options
    plus the new one, then commits the new option's id. Status lists its options under To do,
    In progress and Done headings. Choosing the current option clears it.
  - multiSelect: chips for the chosen options, plus the same picker. Choosing toggles membership,
    and the stored order is the choice order.
  - date: a date input, plus an "End date" toggle that shows a second input, and an "Include time"
    toggle that switches both to `datetime-local`. A day commits `{start: "YYYY-MM-DD"}`. A time
    commits RFC 3339 with the device's offset. An end before the start is refused in place.
    "Clear" commits `null`.
  - checkbox: toggles on click or Space with no editor.
  - url: an inline input. When filled, an "Open link" button opens it through
    `openExternalUrl`, the existing external-link path. Only `http(s)` and `mailto` links open.
  - relation: a page picker over the target database's rows, via `notes/query_database` with
    `query` set to what is typed, limit 20. Choosing toggles a row id.
  - project (system column): the workspace list, as the page's Project property, written through
    `workspaceId` or `clearWorkspace`.
  - chats (system column): read-only links that open each linked chat.
  - created and updated: read-only.
- Escape always cancels and never commits. Blur commits for text, number and url inputs, matching
  the title field.

### The table (`DatabaseTable.tsx`)
- Layout:
  - A sticky header row. `title` is pinned at the left. Then the visible columns, then a trailing
    `+` that opens "Add a property" (a name and a type) and lists hidden columns with "Show".
  - Body rows are windowed. Rows are 36 px. The scroll container's `scrollTop` and `clientHeight`
    pick rows `[floor(top / 36) - 10, ceil((top + height) / 36) + 10]`. Spacer rows above and below
    keep the full height. With 1000 rows and a 600 px viewport, at most 37 rows render.
  - `+ New` below the last row creates a row (a child page) and selects its title cell.
- Selection and keyboard. The grid has one selected cell, `aria-selected` on its `gridcell`.
  - The arrow keys move the selection and stop at the edges.
  - Tab moves right, wrapping to the next row's title. Shift+Tab moves left.
  - Enter opens the selected cell's editor. Typing a printable character on a text, number, url or
    title cell opens the editor seeded with that character.
  - In an editor, Enter commits and selection moves down a row. Escape cancels and keeps the
    selection.
  - Space toggles a checkbox cell.
  - Cmd+Enter (Ctrl+Enter off macOS) on a title cell opens the row page.
  - Clicking a cell selects it. Clicking a selected cell opens its editor, except checkbox, which
    toggles.
- Column header menu (`SchemaMenu.tsx`). Each item calls exactly one method:
  - Rename: `update_schema` `rename`.
  - Type: `update_schema` `retype`. Each type shows `retypeNote` before it is chosen.
  - Edit options (select, multiSelect, status): `update_schema` `setOptions` with the whole list in
    its new order. Each option can be renamed, deleted, or for status regrouped. New options carry no
    id.
  - Sort ascending and Sort descending: `save_view` with `sorts: [{property, direction}]`, every
    other view field kept.
  - Hide: `save_view` with that column's entry `visible: false`, every other field kept.
  - Delete: an in-menu confirmation naming how many loaded rows have a value
    ("Delete Rating? 3 rows have a value."), then `update_schema` `delete`.
  - The title column offers only the sorts.
- Column widths drag from the header's right edge. On release, `save_view` stores the width in that
  column's `properties` entry. A drag that moves less than 2 px saves nothing.
- The table never filters or sorts locally. Rows are shown in the order `notes/query_database`
  returns them.

### Data (`DatabaseView.tsx`)
- Props are the database page's id, an optional `viewId`, and `bounded` (the embed). It reads the
  database through `notes/read_page`, then queries `notes/query_database` with `viewId` set to the
  active view.
- Loading: the first query asks for 100 rows. While `nextCursor` is set, later pages of 500 are
  fetched in the background and appended, so the first paint never waits for the whole table.
- Re-query: after each write, and on every `notes/page_changed` (coalesced to one read per 30 ms).
  A newer query always replaces an older one, and an older answer never lands after a newer one.
- Optimistic cells: a committed value shows at once and stays until the next query answers.
  A refused write puts the cell back and reports the error through `onError`.
- View tabs: one per `database.views`, in order, each with its type's icon. Only `table` views
  render here. Another type shows "Board, calendar and list views arrive in a later update." The
  active view is the `viewId` prop, else the first view.
- An empty database shows the header and "No rows yet" above `+ New`.

### Pages (`NotePageView.tsx`)
- A database page (`page.database` set):
  - Its `DatabaseView` renders above Your words.
  - Your words is the database's description. It renders collapsed to a one-line "Add a
    description" button when empty, and expands on click.
  - The Type property is hidden, since the kind is fixed.
  - The "Pages" list leaves out the database's rows.
- A row page (`page.databaseId` set):
  - A property panel above Your words lists every schema property of its database, in schema
    order, each with its `PropertyCell`, in the existing `Property` row layout.
  - An "Add a property" entry opens the same form as the table's `+`.
  - A commit calls `notes/update_note` with `values: {[id]: value}`, then re-reads the page.
  - The database's schema comes from one `notes/read_page` of `databaseId` when the row opens.

### Creating a database
- The `/` menu gains "Database" (keywords: table, grid, rows, notion). It calls
  `notes/create_database` with `parentId` set to the open page. It inserts
  `<database url="bridge://p/<id>">Title</database>` at the cursor and opens nothing.
- The Notes footer gains "New database" beside "New page". It creates a top-level database titled
  "Untitled database" and opens it.
- The page tree shows a database with a table icon. Its rows nest under it like any child page.

### The `<database>` embed (`blocks.ts`, `views.tsx`)
- `<database url="bridge://p/<id>">Title</database>` alone on a line, with an optional
  ` view="<viewId>"` after the url, parses into a `databaseEmbed` node `{id, view, title}`.
  - It renders back as the same tag, attributes in that order, with the title escaped as
    `pageLink`'s is.
  - `database` stays in `RAW_START`, and the raw-block tokenizer steps aside only when the tag is
    exactly the embed's form. Any other `<database …>` form (different attributes, multiple
    lines) stays a raw block, kept as written.
- An untouched embed round-trips byte for byte through `keepUntouched`.
- The node view renders `DatabaseView` in a frame with a 420 px maximum height, the title, and an
  "Open as page" action. Deleting the block deletes only the block.

### Mock and dev
- `src/notesMock.ts` already serves every database method (#888). Add only a dev seed:
  `seedMockReadingList()` makes "Reading list" with ten rows covering every property type. It is
  called by `seed()` only outside tests (`import.meta.env.MODE !== "test"`), so existing tests'
  seeded pages are unchanged. Tests can call it directly.

## Unit Tests
- `formatValue.test.ts`:
  - `formats every property type and an empty cell`
  - `formats a datetime in the device zone and a range with an arrow`
  - `reads an unknown option or row as empty, never as its id`
  - `orders columns by the view, then schema order, skipping deleted ones`
  - `gives each type its default width and clamps saved widths`
  - `names what every retype keeps`
- `PropertyCell.test.tsx` (jsdom, `react-dom/client` + `act`):
  - `text commits trimmed text and null when emptied`
  - `number refuses NaN and commits a finite number`
  - `select picks, clears and creates an option through setOptions`
  - `status groups its options under their groups`
  - `multiSelect toggles in choice order`
  - `date commits a day, a range and a datetime, and refuses an end before the start`
  - `checkbox toggles on click and Space`
  - `url opens only http, https and mailto links through openExternalUrl`
  - `relation searches the target database and toggles a row`
  - `Escape cancels every editor without committing`

## Integration / Functional Tests
- `DatabaseTable.test.tsx`:
  - `renders the title and visible columns in view order with their cells`
  - `windows 1000 rows to the visible slice and keeps the full scroll height`
  - `moves the selection with arrows and Tab, and stops at the edges`
  - `Enter and typing open an editor; Enter commits and moves down; Escape keeps the old value`
  - `Cmd+Enter on a title opens the row page`
  - `header menu renames, retypes, edits options and deletes through update_schema`
  - `sort and hide save the view and keep its other fields`
  - `a dragged column width saves to the view; a click saves nothing`
  - `+ New makes a row and selects its title`
  - `+ shows a hidden column again`
- `DatabaseView.test.tsx`:
  - `loads the first 100 rows, then the rest by cursor`
  - `shows a committed value at once and puts it back when the write is refused`
  - `re-queries on page_changed, and an older answer never replaces a newer one`
  - `shows a later-update notice for a non-table view`
- `NotePageView` via `NotesScreen.test.tsx`:
  - `a database page shows its table above a collapsed description`
  - `a row page lists its properties and an edit there updates the table`
  - `New database makes a database and the tree shows it with a table icon`
- `NoteEditor.test.tsx`:
  - `a database block round-trips byte for byte beside every other block`
  - `a database tag with other attributes stays raw`
  - `the / menu's Database makes a database under the page and inserts its block`

## Smoke Tests
- `bun run build` and `bun run test` are green.
- `bun run dev`: Notes shows "Reading list" with a table icon, it opens as a table of ten rows, and
  `/database` in a page inserts a working embedded table.

## E2E Tests
- N/A as automated tests: no Notes E2E harness exists.
- The acceptance flow is run by hand in `bun run dev` (mock) and in the release app (real core),
  and recorded in the PR:
  1. Run `/database` in a page.
  2. Add a property of each type.
  3. Edit a cell of each type.
  4. Reload and check the values.
  5. Open a row.
  6. Edit a value in the row panel and check the table.

## Manual / cURL Tests
- Performance, release app: seed 1000 rows into a database through the agent tools or a script
  against `bridged`, open it, and read the `notes.table.first-paint` mark, which `DatabaseView`
  records with `performance.mark` and `performance.measure` from the first query to the first
  painted row. It must be under 200 ms. Scrolling stays at 60 fps in the WebKit timeline.
- Screenshots in the PR body, dark and light: the table, a header menu, an open select picker, and
  a row page.
