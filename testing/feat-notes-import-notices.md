# feat/notes-import-notices: Test Contract

Issue #894 (parent #886). Builds on #887's importer and frontmatter projection and #889's row
page.

When a row's `.md` file says something its property cannot read (`Rating: three` in a number
column), the importer keeps the line verbatim and writes nothing. Until now the only trace was a log
line, and the next projection replaced the line. This branch records a notice the row page shows.

## Functional Behavior

### Storage (`notes/store.rs`, schema 5)
- `page_notices(space_id, page_id, kind, property, raw, expected, seq, created_at)`, with primary key
  `(space_id, page_id, kind, property)`, so a page holds at most one notice per kind and property.
- `kind` is `unreadableValue`. `property` is the property id. `raw` is the text after the key, as
  the file had it. `expected` is the property's type (`number`, `select`, …). `seq` is the page's
  `seq` when the notice was written.
- Migration 5 creates the table on an existing store and leaves every other table as it was.

### Writing notices (`notes/projector.rs::apply_import`)
- On a row's import, each frontmatter line that names a property but does not parse or validate
  records an `unreadableValue` notice for that property. A second import of the same file replaces
  the notice and never adds a second one.
- The same import clears the notice of every other property of that database: a line that parses
  now, a line that was removed, a property that is gone.
- The log line stays.

### Clearing notices (`notes/service.rs`)
- Setting a cell from Bridge, through `set_values` or `update` with `values`, clears that property's
  notice on that page in the same write.
- Deleting a page deletes its notices.
- `rebuild_index` deletes every notice. Notices are not part of the change log, so they cannot be
  rebuilt from it. The next import of a still-unreadable file records them again.

### Reading notices
- `NotePage.notices: NoteNotice[]` on the wire, newest first. `NoteNotice` has `kind`, `propertyId`,
  `raw`, `expected`, `seq` and `createdAt`. Empty means none, and it is left out of the JSON when
  empty.
- Only a row of a database can hold notices, so `read` queries `page_notices` only for a row. Any
  other page costs no extra query.

### The row page (`RowProperties.tsx`)
- Each notice shows above the properties as one line, for example "Rating in the file could not be
  read as a number". The raw text follows in monospace (`three`), and the line carries
  `role="status"`.
- A notice for a property the schema no longer has is not shown.
- When the page is re-read without the notice (the cell was set, or the file was fixed), the line
  goes away.
- `src/notesMock.ts` returns `notices: []` on every page: it has no file importer.

## Unit Tests
- `store::tests::migrates_a_v4_store_to_page_notices`
- `projector::tests`:
  - `an_unreadable_value_records_one_notice`
  - `a_second_import_does_not_duplicate_a_notice`
  - `fixing_the_line_clears_the_notice`
- `service::tests`:
  - `setting_the_cell_clears_its_notice`
  - `rebuild_index_clears_notices`
  - `deleting_a_page_deletes_its_notices`
  - `only_a_row_reads_notices`

## Integration / Functional Tests
- `RowProperties.test.tsx`:
  - `shows a notice for a line the file could not read`
  - `hides a notice once the page is re-read without it`
  - `skips a notice for a property that is gone`

## Smoke Tests
- `bun run build`, `bun run test` (vitest and `cargo test --workspace`), the protocol artifact drift
  test, and `check:visual` are green.

## E2E Tests
- N/A as automated tests. By hand against the real core:
  1. Edit a row's file to `Rating: three` and watch the notice appear.
  2. Set Rating in Bridge and watch it go.

## Manual / cURL Tests
- Covered by the Rust `projector::tests`, which write real files and run `import_changes`.
