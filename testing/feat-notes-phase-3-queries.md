# feat/notes-phase-3-queries: Test Contract

Issue #888 (parent #886, epic #865). Builds on the data model from #887 (merged in #892):
database pages, `Y.Map` schema/views/values, the `database_props` and `page_values` index, and
the system database. This branch adds the query planner, the wire methods, agent property
filters, and the mock that the frontend issues (#889, #890) build on.

## Functional Behavior

### Query semantics (`notes/query.rs`, mirrored exactly by `mockQueryDatabase`)
- Rows of a `children` database are its live child pages. Rows of the `space` (system)
  database are every live page in the space except the system database itself.
- A filter is `{and: [...]}`, `{or: [...]}` or a condition `{property, op, value?}`. Groups
  nest at most two levels (a group inside a group, no deeper) and hold at most 20 conditions
  in total. An empty group matches every row.
- `property` is a property id from the schema or a system id: `title`, `kind`, `state`, `tags`,
  `project`, `chats`, `parent`, `created`, `updated`, `createdBy`.
- Ops by type. A negated op is exactly NOT its positive op, so an empty cell matches every
  negation (`isNot`, `notContains`, `notEquals`, `ne`, `isNoneOf`):
  - text, url, `title`, `createdBy`: `contains`, `notContains`, `equals`, `notEquals`,
    `startsWith`, `isEmpty`, `isNotEmpty`. Comparisons fold ASCII case only (SQLite
    `lower()`), and `""` counts as empty.
  - number: `eq`, `ne`, `lt`, `lte`, `gt`, `gte`, `isEmpty`, `isNotEmpty`. An empty cell
    fails every ordered comparison.
  - date, `created`, `updated`: `before`, `after`, `onOrBefore`, `onOrAfter`, `equals`,
    `isEmpty`, `isNotEmpty`, `within`. Every date comparison is by day: a `date` cell's day,
    a `datetime` cell's UTC day, and a filter value of `YYYY-MM-DD` or RFC 3339 normalized to
    its UTC day. `within` takes `pastWeek` (today-7 … today), `nextWeek` (today … today+7),
    `thisWeek` (the ISO week, Monday … Sunday, holding today), `thisMonth` (the calendar month),
    `pastMonth` (today-30 … today) or `nextMonth` (today … today+30), all inclusive. `today` is
    `YYYY-MM-DD` from the caller (the device's local day), and defaults to the UTC day.
  - select, status, `kind`, `state`, `project`: `is`, `isNot`, `isAnyOf`, `isNoneOf`,
    `isEmpty`, `isNotEmpty`, plus `inGroup` (`todo`, `inProgress`, `done`) for status.
    Values are option ids (or the raw `kind`/`state`/workspace id).
  - multiSelect, relation, `tags`: `contains`, `notContains`, `containsAny`, `isEmpty`,
    `isNotEmpty`.
  - checkbox: `is` with `true` or `false`; an empty cell is `false`.
  - `chats`: `isEmpty`, `isNotEmpty`. `parent`: `isEmpty`, `isNotEmpty`, `equals`,
    `notEquals` (a page id).
- An unknown property, an op the type does not take, or a value of the wrong shape fails with
  `invalid_params`, naming the valid properties or ops. Nothing is read.
- Sorts: up to five `{property, direction: asc|desc}`, on text, url, number, date, checkbox,
  select and status (by option order) and the system `title`, `kind`, `state`, `project`,
  `created`, `updated`, `createdBy`. Lists (multiSelect, relation, `tags`) and `chats`/`parent`
  are refused. Empty values sort last in both directions. The final tiebreak is
  `updated desc`, then `id desc`, so every order is total and stable.
- `groupBy {property, dateBucket?}` on select, status, multiSelect, checkbox, `kind`, `state`,
  `project` and dates (`day`, `week` keyed by its Monday, `month` as `YYYY-MM`):
  - Option types list every option in option order, count 0 included, then "No value" (key
    `""`). `kind` and `state` list their fixed values the same way. Checkbox is `true` then
    `false`, and a missing cell counts as `false`.
  - A multiSelect row appears in each of its options' groups, and an empty one in "No value".
  - `project` and dates list only keys that occur, ascending, then "No value" when it occurs.
  - Each group carries `{key, label, count, rows}` with its first `groupLimit` rows (default
    50, max 200) in the query's sort order.
- `query` (free text) intersects with `pages_fts` through `links::fts_query`.
- Paging: `limit` (default 100, max 500) and an opaque `cursor`. The cursor holds the last
  row's sort keys and id, not an offset: the next page starts strictly after that position
  in the sort order, so a row inserted or deleted between reads is neither skipped nor shown
  twice. `total` counts every matching row. `nextCursor` is absent on the last page.
- A saved `viewId` supplies the view's filter, sorts and grouping. Any of `filter`, `sorts` or
  `groupBy` given explicitly replaces the view's for that call only.
- Filtering happens in SQL: one statement over `pages`, with one `EXISTS` per condition on
  `page_values` (using the `(space_id, database_id, prop_id, *_v)` indexes) and every value
  bound as a parameter. The sort keys are SQL expressions too. The cursor comparison and the
  grouping run over the ordered `(keys, id)` list SQL returns, never over unfiltered rows.

### Wire methods (all through `bridge_core::api`, `bridged`, the Tauri shell and `src/api.ts`)
- `notes/create_database {title?, parentId?, workspaceId?, properties?, views?}` → `NotePage`.
  With no `properties` it gets one Status (status) property with To do / In progress / Done.
  With no `views` it gets one "Table" view.
- `notes/update_schema {id, changes}` → `NoteDatabase`. A change is one of:
  - `{op: "add", property}`
  - `{op: "rename", propertyId, name}`
  - `{op: "retype", propertyId, type}`
  - `{op: "delete", propertyId}`
  - `{op: "setOptions", propertyId, options: [{id?, name, color?, group?}]}`, which replaces
    the whole option list. An entry without an id is new, a missing id is deleted (its values
    are cleared), and order is list order.
  - `{op: "reorder", propertyIds}`, which sets the schema order. It must name every property
    once.
  
  The batch is validated and the schema committed in one transaction, under #887's rules. Row
  conversions and option clears follow as attributed row writes. #887 documents that a
  failure after the schema commit leaves the schema changed, and a re-run repairs it.
- `notes/save_view {databaseId, view}` → `NoteDatabaseView` (no `view.id` creates, an id
  replaces). `notes/delete_view {databaseId, viewId}` refuses to delete the last view.
- `notes/query_database {databaseId, viewId?, filter?, sorts?, groupBy?, groupLimit?, query?,
  today?, limit?, cursor?}` → `{rows, groups?, total, nextCursor?}`. A row is
  `{note: Note, values}`, with values keyed by property id. This deliberately differs from the
  issue's `Note & {values}`: a nested `note` keeps the generated TypeScript a plain interface.
- `UpdateNoteParams.values: {propertyId: value | null}` sets or clears a row's cells through
  `set_values` (`null` clears).
- `NotePage` gains `database?: {scope, properties, views}` on a database page, and
  `values?` and `databaseId?` on a row.
- `NotesList` gains `systemDatabaseId`.

### Agent tools
- `notes_search` gains three arguments: `database` (an id, or a title resolved like `[[title]]`),
  `where` (the filter grammar, with property names in place of ids and option names in place of
  option ids) and `sort`. Results include `properties` by name for rows. Without `database`,
  `where` and `sort` address the system properties over every page.
- `notes_read` on a database returns `database: {properties: [{name, type, options}], views: [names]}`
  and its first 50 rows as a Markdown table. On a row, it returns `properties` by name.
- `notes_create` gains three arguments: `database` (makes the page a row of that database),
  `properties` (values by name) and `asDatabase: {properties: [{name, type, options?,
  database?}]}`, which makes a database with that schema (`database` names a relation's
  target).
- `notes_set` gains `properties` by name; `null` clears.
- An unknown database, property name, option name or invalid value fails with status 400. The
  message lists the valid names or options, and nothing is written. Agents can't change a
  schema after creating it.
- The access matrix is unchanged. Tool names are unchanged, so `sidecar/claude-agent/options.mjs`
  (an allowlist of names) and the OpenCode tool list need no change. The new arguments live in
  the schemas the `bridge` MCP server serves from `mcp_apps/notes_tools.rs`.
- The notes purpose prompt (`notes/agent.rs`) says databases exist and to filter with `where`
  instead of reading every row.

## Unit Tests (Rust, `cargo test -p bridge-core --lib notes::`)
- `query::tests`:
  - `every_op_matches_a_reference_filter`: a 300-row randomized fixture (fixed seed) holding
    every property type. Each op on each type, plus nested `and`/`or`, returns exactly the ids
    a plain in-memory reference evaluator returns.
  - `a_bad_filter_names_the_valid_choices`: an unknown property, an unknown op, a wrong value
    shape, depth 3 and 21 conditions are each refused with the valid choices named.
  - `values_are_bound_never_interpolated`: a property `'; DROP TABLE pages; --` is refused as
    unknown, and a text value of `' OR 1=1 --` matches only literally.
  - `every_condition_uses_an_index`: `EXPLAIN QUERY PLAN` for a query with one condition per
    type shows `page_values` reached only through its indexes, never a full scan.
  - `sorts_are_total_and_empty_values_sort_last`: covers both directions, the tiebreak, and
    option order.
  - `a_cursor_neither_skips_nor_repeats`: page through 37 rows by 10; insert and delete rows
    between reads; every surviving row is seen exactly once, in order.
  - `groups_follow_option_order_and_multi_select_rows_repeat`: covers counts, the "No value"
    group, checkbox groups, date buckets and `groupLimit`.
  - `within_uses_the_callers_today`.
  - `the_system_views_return_todays_lists`: each seeded system view queried by `viewId`
    returns exactly the port of today's `NotesScreen` filter on a fixture space.
  - `performance_gate`: 1000 rows (`database::tests::seed_rows` plus Status and Due), three
    conditions, two sorts and a groupBy. The best of five must be under 200 ms in a release
    build (`cargo test --release -p bridge-core --lib performance_gate`). A debug build only
    asserts under 2 s, so a slow CI debug run doesn't flake.
  - `the_shared_fixture_matches`: loads `testing/fixtures/notes-query-cases.json`, seeds it,
    and checks every case's ordered titles.
- `service::tests`:
  - `create_database_seeds_status_and_a_table_view`.
  - `set_options_diffs_into_add_rename_delete_and_order`.
  - `reorder_must_name_every_property`.
  - `the_last_view_cannot_be_deleted`.
  - `read_carries_the_database_or_the_row_values`.
  - `update_with_values_sets_and_clears`.
- `tool::tests`:
  - `search_filters_a_database_by_names`.
  - `unknown_names_and_options_list_the_valid_ones`: covers the database, a property, an
    option and an invalid value; nothing is written.
  - `create_makes_rows_and_databases`.
  - `set_writes_properties_by_name`.
  - `read_shows_schema_rows_and_row_properties`.
  - `read_only_chats_can_filter_but_not_write`.

## Integration / Functional Tests
- `bridge-protocol`:
  - Regenerated artifacts pass `tsgen::checked_in_artifacts_match_the_contract`.
  - Every new params type is `deny_unknown_fields` and named after its method.
  - `protocol_mirror` maps the core `PropertyDef` and `ViewDef` onto their wire mirrors.
- `bridged`: each new method round-trips over the socket, as the existing notes methods do.
- `bridge-deck`: the Tauri command signatures match their params, and the registry and handler
  are 1:1.

## Smoke Tests
- `bun run check` (tsc plus `cargo check --workspace`), `bun run build`.

## E2E Tests
N/A: no UI in this branch (#889, #890). The mock path is covered by vitest below.

## Vitest
- `src/notesMock.test.ts`:
  - `mock queries match the shared fixture`: the same `notes-query-cases.json` cases, run
    through `mockQueryDatabase`, give the same ordered titles as the Rust planner.
  - The mock database methods: create, update schema, save and delete a view, values through
    `mockUpdateNote`, the system database hidden from the list, and `systemDatabaseId`.
- `src/api.test.ts` (or the nearest existing api test): the new `bridgeApi` methods call their
  wire method names.

## Manual / cURL Tests
- Run the daemon and call `notes/create_database`, `notes/update_schema`, `notes/query_database`
  with `bridge exec --json` or the socket. Not required for merge: the `bridged` round trip
  covers it.

## Amendments after review (PR #895)

Each item below came out of review and has a regression test that fails without its fix.

- **Option cleanup is repaired on retry.** `setOptions` and `deleteOption` clear every row value that names an option the property no longer has, on every run, including a run that changes no schema. A retry after an interrupted cleanup finishes it.
- **Saved views follow schema edits.** The same write that changes a schema also prunes every view against the new schema:
  - A condition naming a removed property or option is dropped. A list keeps its surviving options and is dropped only when none are left. A group that empties is dropped.
  - A condition whose op the retyped property no longer takes is dropped.
  - Sorts, the grouping, the date property and visible-property entries that no longer apply are removed.
  
  A saved view is therefore always queryable.
- **`notes_search` keeps the caller's full filter allowance.** Its implicit constraints (`kind`, `state`, `tags`, archived pages left out) compile beside the caller's `where` and count toward neither its two-level depth nor its 20 conditions.
- **Tags are normalized everywhere.** Tag values are trimmed, have their `#` dropped and are folded, in the tool's top-level `tags` and in every `tags` condition, exactly as `notes/list_notes` treats them.
- **Free text with no word token searches literally.** A non-empty `query` with no word token (for example `🤖`) matches title or body text by substring with ASCII case folding, the same fallback `notes/list_notes` uses. It is never ignored.
- **Initial views can name new properties.** A `create_database` view may name a property being created, and its options, by name. They resolve to the generated ids inside the creating write.
- **A row and its first cells are one write.** `notes_create` with `database` and `properties` makes the row and its cells in one write. A refused cell makes nothing.
- **`asDatabase` keeps what else was passed.** It keeps `tags` and `content` in the same creating write, and the response is the note as stored.
- **`today` is a real day.** It is `YYYY-MM-DD` with a four-digit year. Relative ranges and day bounds use checked arithmetic, and a date out of range is refused with `invalid_params`. Nothing panics while the notes database lock is held.
- **The mock matches the core in two more places.** A same-type retype keeps valid cells and repairs stranded ones by their shape, as the core does. Initial views are validated against the draft schema before the database is made.
- **`groupBy: null` is the same as leaving `groupBy` out.** A `viewId` supplies each field the call leaves out, and the wire can't tell `null` from absent. To run a saved view without its grouping, query without `viewId` and pass the view's filter and sorts. An explicit `sorts: []` and an explicit `filter: {and: []}` already replace the view's.
