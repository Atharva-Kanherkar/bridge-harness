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
  property id to `PropertyDef`; `Y.Map("views")` maps view id to `ViewDef`. One entry per key, so
  two people editing different columns or views merge cleanly.
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
  `multiSelect`: an array of option ids. `date`: `{start, end?}` where each is `YYYY-MM-DD` or
  RFC 3339. `checkbox`: a boolean. `relation`: an array of page ids.
- A value for an unknown property id, an unknown option id, the wrong JSON type or a non-finite
  number is refused and writes nothing.
- A value whose property was deleted stays in the doc but is not indexed or projected.

### Retype conversion
- number↔text; select↔text (matched to an option by name, or a new option is created);
  select→multiSelect; checkbox↔text (`"true"`/`"false"`). Every other pair clears the value.
- Retyping to `relation` is refused: a relation has to name the database it points at, and a
  conversion cannot invent one. Retyping to `status` gives every option a group; retyping away
  from `status` drops them.
- Each converted row is one attributed write, so `page_history` can restore the old values.

### Index maintenance
- `database_props` and `page_values` match the docs after every write: set, clear, row
  reparenting, property deletion and retype.
- `text_v` holds text, url, option id, `"true"`/`"false"`, or the related page id; `num_v` the
  number; `date_v` the normalized start date. One row per scalar item, `item` being 0 for scalars
  and the array index for `multiSelect` and `relation`.
- Dropping both tables and calling `rebuild_index` restores them exactly.

### System database
- Every space has exactly one database page with `scope: "space"`, title "Notes" and
  `created_by = "system"`, created idempotently on open and recorded in `spaces.system_database_id`.
  An unclaimed one left by a crash is adopted on the next open rather than duplicated.
- It is hidden from `notes/list_notes` (and therefore from the page tree).
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
- A key that names no property stays in `Frontmatter.unknown` verbatim, and a foreign key survives
  every rewrite. A value that does not parse is logged and the document's value wins on the next
  projection, exactly as an invalid `state:` or `kind:` already behaves; it writes nothing.
- A rename rewrites the key in every row's file and writes no row doc. A renamed option rewrites
  the files that use it.
- `database` is a reserved frontmatter key: a property cannot be named it.

## Unit Tests

Rust, `#[cfg(test)]` beside the code:

- `store::tests`: the migration from a populated v3 fixture, the v4 index list, the new `kind`
  check, and the refusal of a v5 file.
- `doc::tests`: records replay, one map entry per property/view, concurrent schema edits to
  different properties.
- `database::tests`: value validation for every type; retype conversion for every listed pair;
  the name rules; `page_values` maintenance on set, clear, reparent, delete and retype;
  `rebuild_index` parity; the seeded system views and the hidden system database; `seed_rows`
  used by #888's performance gate.
- `markdown::tests`: `database:` and property lines round-trip, unknown keys survive.
- `projector::tests`: a frontmatter round trip with every type; a relation title resolving; an
  import of a hand-edited row file including invalid values kept as unknown; a rename rewriting
  files but not docs.

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
