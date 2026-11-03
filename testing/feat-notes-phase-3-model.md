# feat/notes-phase-3-model — Test Contract

Bridge Notes phase 3, backend data model. Issue #887, parent #886, epic #865. Locked before
implementation. No UI and no new `notes/*` protocol methods in this branch: everything here is
reachable through `NotesService`, the `notes.db` schema, or the Markdown projector.

## Functional Behavior

### Store (migration 4)
- `notes.db` at schema 3 opens, migrates to 4, and keeps every `pages`, `page_updates`,
  `page_snapshots`, `structured_views`, `model_jobs`, `pages_fts`, `pages_fts_map`, `page_links`
  and `assets` row. `idx_pages_space_state`, `idx_pages_parent` and `idx_pages_link_key` exist
  after the rebuild.
- `pages.kind` accepts `note`, `idea`, `task`, `question`, `database` and refuses anything else.
- A file at schema 5 is refused with the existing "newer than this build understands" error.
- New tables: `database_props(space_id, database_id, prop_id, name, type, def)` keyed by
  `(space_id, database_id, prop_id)`, and `page_values(space_id, database_id, page_id, prop_id,
  item, text_v, num_v, date_v)` keyed by `(space_id, page_id, prop_id, item)`, with indexes on
  `(space_id, database_id, prop_id, num_v | text_v | date_v)`.
- `spaces` gains `system_database_id`.

### Doc keys (one `Y.Map` per kind of entry)
- Database page: `Y.Map("database")` holds `scope` (`children` or `space`); `Y.Map("schema")` maps
  property id to `PropertyDef`; `Y.Map("views")` maps view id to `ViewDef`; `Y.Map("retired")` maps
  an old property name to the id that owned it, so a rename reaches files that still use it. One
  entry per key, so two people editing different columns or views merge cleanly. Options live inside
  their property's entry: two people changing the same property at once are last-writer-wins.
- Row page: `Y.Map("values")` maps property id to a JSON value. A row belongs to the database it
  is the child of (`parent_id`).
- Replaying `page_updates` reproduces the database meta, every property, every view and every
  value, so `rebuild_index` restores them with no other source.

### Property and view shapes
- `PropertyDef`: `{name, type, order, options?: [{id, name, color?, group?}], relation?:
  {databaseId}, dateFormat?: "date" | "datetime"}`. `group` exists only on a `status` option and
  is `todo`, `inProgress` or `done`.
- Types: `text`, `number`, `select`, `multiSelect`, `date`, `checkbox`, `url`, `relation`,
  `status`. System property ids `title`, `kind`, `state`, `tags`, `project`, `chats`, `created`,
  `updated`, `createdBy` are addressed by views and are never written to `values`.
- `ViewDef`: `{name, type, order, filter?, sorts?, groupBy?, dateProperty?, properties?}` with
  `type` one of `table`, `board`, `calendar`, `list`. The filter is stored as JSON and only has to
  parse here; filtering is #888.
- A property name may not equal a system id or a Frontmatter `KNOWN` key, case-insensitively, and
  names are unique within a database case-insensitively.

### Values
- `text`, `url`: a JSON string. `number`: a finite JSON number. `select`, `status`: an option id.
  `multiSelect`: an array of unique option ids. `date`: `{start, end?}` where each is `YYYY-MM-DD`
  or RFC 3339 and `end` is not before `start`. `checkbox`: a boolean. `relation`: an array of
  unique page ids, each a live row of the database the property names.
- A value for an unknown property id, an unknown option id, the wrong JSON type, a non-finite
  number or a repeated list item is refused, and a refused patch writes nothing.
- A value whose property was deleted stays in the doc but is not indexed or projected.
- A cell cleared in Bridge records the value it removed on the row (a `cleared` map) until a
  projection has dropped the stale line, so a clear survives a file that has a pending outside
  edit. The line leaves only while it still says the cleared value; a different value on that
  line is the person's and imports. Import compares a line with the file's base, not with the
  cleared value, so the same value typed back in after the line left imports like any edit.

### Retype conversion
- number↔text; select↔text (matched to an option by name, or a new option is created);
  select→multiSelect; checkbox↔text (`"true"`/`"false"`). Every other pair clears the value.
- Retyping to `relation` is refused: a relation has to name the database it points at, and a
  conversion cannot invent one. Retyping to `status` gives every option a group; retyping away
  from `status` drops them.
- Each converted row is one attributed write, so `page_history` can restore the old values.
- A batch may change a property once, except option edits, which fold into one def in order.
  Validation runs inside the write's transaction, so two callers cannot add the same name; the
  schema commits first and the row writes follow, so a failure after the commit leaves the schema
  changed and returns the error. Re-running the same retype repairs the rows it did not reach:
  a value that does not fit the current type is converted by its own JSON shape where it can
  (a number in a text column becomes its text) and cleared otherwise, and the file is rewritten.

### Index maintenance
- `database_props` and `page_values` match the docs after every write: set, clear, row
  reparenting, property deletion and retype.
- `text_v` holds text, url, option id, `"true"`/`"false"`, or the related page id; `num_v` the
  number; `date_v` the UTC day of the instant for a `date` and the instant itself for a
  `datetime`. A `date` value holding a time is placed on its UTC day, so a calendar view (#890)
  must use the instant rather than the indexed day when it shows one. One row per scalar item,
  `item` being 0 for scalars and the array index for `multiSelect` and `relation`.
- Writing a database page itself (a rename, a view) never touches its rows' indexed values.
- Dropping both tables and calling `rebuild_index` restores them exactly, for live pages only: a
  deleted database or row leaves nothing behind.

### System database
- Every space has exactly one database page with `scope: "space"`, title "Notes" and
  `created_by = "system"`, created idempotently on open and recorded in `spaces.system_database_id`.
  An unclaimed one left by a crash is adopted on the next open rather than duplicated.
- It is a record, not a page: no Markdown file, no `link_key`, no search row, no link edges, and
  `[[Notes]]` resolves to a person's page. It is hidden from `notes/list_notes` (and therefore
  from the page tree), and Bridge refuses to delete, archive, retitle, edit or reparent it, or to
  put a page under it, whether by `create` or by a move. Its views are the person's: `put_view`
  and `delete_view` work on it.
- Only `create_system_database` may use `scope: "space"`; a caller that asks for it is refused.
- It is seeded with these views, reproducing today's `NotesScreen` lists:

| View | Predicate |
| --- | --- |
| All | `state ≠ archived` and (`kind ≠ task` or `parent` is empty) |
| Inbox | `state = inbox` |
| Pages | `kind ≠ task` and `state ≠ archived` |
| Tasks | `kind = task` and `state ≠ archived` |
| Ideas | `kind = idea` |
| In progress with an agent | `state = doing` and `chats` is not empty |
| Archived | `state = archived` |

- A view executor hides the system database the same way `notes/list_notes` does, from
  `spaces.system_database_id`, because a saved filter cannot express "this is the list's own
  database". #888 owns that executor.

### Markdown
- A row's file writes one frontmatter line per non-empty schema property, by name, after the
  `KNOWN` keys and in schema `order`. Values use their natural form: `select` and `status` the
  option name, `multiSelect` an array of names, `relation` an array of `"[[Title]]"` (or the page
  id when the title is missing or not unique inside the related database), `date` `"2026-11-10"`
  or `{"start":…,"end":…}`.
- A database page's file writes `kind: "database"`, then one `database:` line holding the JSON of
  `{scope, schema, views}`.
- Import maps frontmatter names to property ids by name, case-insensitively, against the row's
  database schema, and parses each value by type. It compares the file with the page as the file
  was written from, so a value set in Bridge while an outside edit waits is not reverted.
- A property the page had a value for and the file no longer names is a clear. A line that
  predates the property has no base value, so it stays as a foreign key; that is also true when
  the value cannot be parsed for the type the new property has, because it is the person's only
  copy until they set the cell.
- Every foreign key survives every rewrite, including a `database:` key on a page that is not a
  database (which keeps its place) and an old name reused later by another tool. A value that
  does not parse for a property the document already has is logged and the document's value wins
  on the next projection, exactly as an invalid `state:` or `kind:` already behaves; it writes
  nothing. There is no `page_history` notice yet; the log line is the notice this phase has, and
  the follow-up issue tracks the surface.
- Adding or renaming a property imports a line the rows' files already had for that name, so an
  existing key becomes the cell instead of being dropped.
- A rename rewrites the key in every row's file and writes no row doc. The rewrite runs on the
  maintenance pass (the rows are marked stale), not inside the request. A stale line goes only
  when the document writes the same value under the property's new name; a line the person edited
  is kept as a foreign key. A renamed option does the same.
- Deleting a property leaves its lines in the row files as foreign keys, which keeps the data.
  Re-adding a property with that name starts empty and does not import them.
- `database` is a reserved frontmatter key: a property cannot be named it.

## Unit Tests

Rust, `#[cfg(test)]` beside the code:

- `store::tests`: the migration from a populated v3 fixture, the v4 index list, the new `kind`
  check, and the refusal of a v5 file.
- `doc::tests`: records replay, one map entry per property/view, concurrent schema edits to
  different properties, and last-writer-wins for two edits to one property.
- `database::tests`: value validation for every type (dates, duplicates, ranges); retype
  conversion for every listed pair; the name rules including YAML scalars and one Unicode fold;
  `page_values` maintenance on set, clear, reparent, delete and retype; `rebuild_index` parity
  including deleted pages; the seeded views evaluated against a fixture space and compared with
  a port of today's `NotesScreen` filters; `seed_rows` used by #888's performance gate.
- `markdown::tests`: `database:` and property lines round-trip, unknown keys survive.
- `projector::tests`: a frontmatter round trip with every type; a relation title resolving or an
  id when it is ambiguous; an import of a hand-edited row file; a foreign `database:` key kept on
  a plain page; a rename rewriting files but not docs.
- `service::tests`: the system database's guards, no file and no link target; writes to a
  database keeping its rows indexed; concurrent adds of one name leaving one property; a batch
  touching one property twice refused; relations naming only rows of their database; a deleted
  database and a deleted row leaving no index after a rebuild.

## Integration / Functional Tests

- A v3 `notes.db` copied into a data directory opens through `NotesService::open` and keeps its
  pages and files.
- Setting values through `set_values` and importing the same value from the file end in the same
  doc state.

## Smoke Tests

- `cargo test -p bridge-core notes::` green.
- `cargo check --workspace` green.
- `bun run test` green (no frontend change in this branch; guards against protocol churn).

## E2E Tests

N/A. No UI and no protocol method in this branch.

## Manual / cURL Tests

N/A. Every behavior is covered by Rust tests; the wire surface arrives with #888.
