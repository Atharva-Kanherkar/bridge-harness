# feat/notes-phase-2-backend Test Contract

Source: GitHub issue #879, the backend slice of #878. Frontend editor and sidebar behavior are out of scope.

## Functional Behavior

### Schema v3 and migration
- Bump `notes.db` to schema v3 without changing existing v1/v2 data.
- Add `pages_fts` using FTS5 with `page_id UNINDEXED`, `space_id UNINDEXED`, `title`, `body`, and `tags`, using the `porter unicode61` tokenizer.
- Add `page_links` with the exact columns, primary key, checks, and indexes specified in #879.
- Populate both new structures from existing live pages during migration and whenever the index is rebuilt.

### Full-text search
- Index the live title, body, and tags on every write; remove deleted pages from `pages_fts`.
- A non-empty query uses safely quoted FTS5 prefix tokens and `bm25(pages_fts, 0.0, 0.0, 5.0, 1.0, 2.0)`, then `pages.updated_at DESC`.
- Title hits rank above body-only hits and Porter stemming works. The two initial zero weights account for the unindexed identity columns; the indexed title/body/tags weights remain 5/1/2.
- A punctuation-only query with no FTS tokens falls back to the existing substring `LIKE` behavior.
- `rebuild_index` reconstructs the ordinary page index, FTS index, and link graph consistently.

### Links and backlinks
- Parse `[[Target title]]`, `[[Target title|Display text]]`, and `<page url="bridge://p/<id>">Title</page>` outside fenced and inline code.
- Trim wiki targets and ignore empty or newline-spanning targets. Preserve aliases when renaming.
- Resolve wiki targets case-insensitively against live page titles, or the derived first-line display title when `title IS NULL`, in the same space.
- Resolve page tags only to a live page with the matching id in the same space.
- Re-resolve previously unresolved links when a matching page is created or titled.
- Store a control-syntax-stripped excerpt from the surrounding line, clipped to 200 characters.
- `NotePage.backlinks` returns live, non-self incoming links ordered by source `updated_at DESC`, with page id, title, kind, excerpt, and timestamp.

### Rename propagation
- When a titled page is renamed, rewrite matching incoming wiki links and child-page tags outside code.
- Rewrite both `[[Old]]` and aliased `[[Old|Alias]]` forms, and escape the new title inside `<page>` tags.
- Route each affected page through `NotesService::write` with the original actor so history, indexing, and Markdown disk mirrors stay in sync.

### Parent moves
- Extend protocol and service update inputs with `parent_id` and default-false `clear_parent`.
- Support moving a page under a live, non-task parent and clearing its parent to return it to root; promotion also moves an `inbox` note to `open` as specified in #879.
- Reject missing, deleted, task, self, and descendant parents. Walk up to 64 ancestors and return `BridgeError::Invalid` for cycles.

### Compatibility and scope
- Preserve the existing single write path and untouched Markdown behavior.
- Regenerate protocol artifacts and update affected TypeScript mocks/tests.
- Do not implement wiki autocomplete, backlink panels, sidebar tree UI, databases, or other #880/Phase 3 work.

## Unit Tests

Add focused coverage in `notes/links.rs`, `notes/service.rs`, and `notes/store.rs` as appropriate:
- Wiki links, aliases, child-page tags, inline/fenced code exclusion, excerpts, and rename rewriting.
- v1 and v2 migration to v3, including FTS and link backfill.
- `fts5_search_ranks_title_and_stems_words`, including prefix search and punctuation-only fallback.
- `wiki_links_and_page_tags_populate_backlinks_and_resolve_late_pages`.
- `renaming_a_page_updates_wiki_links_and_page_tags_in_backlinked_pages`, including history and disk mirrors.
- `reparenting_a_page_moves_it_and_refuses_cycles`, including invalid parents and root clearing.
- `rebuild_index_restores_fts_and_backlinks`.
- Link indexing never parses links inside code fences or inline code.

## Integration / Functional Tests

- Add daemon round trips in `src-tauri/bridged/tests/notes.rs` for backlinks, rename propagation, and `parentId`/`clearParent` updates.
- Verify generated protocol artifacts have no drift and TypeScript mocks satisfy the generated types.
- Verify person, agent-tool, and file-import writes all pass through the same index maintenance path where existing coverage supports it.

## Smoke Tests

- A fresh notes database initializes at schema v3.
- Create, read, search, rename, reparent, and rebuild operations succeed against a temporary notes root and database.
- Existing Notes tests remain green.

## E2E Tests

- Backend daemon integration covers the issue #879 slice end to end.
- UI autocomplete, backlinks panel, sidebar tree, image resizing, and WKWebView latency checks are N/A because they belong to #880.

## Manual / cURL Tests

- No HTTP cURL checks are required; Notes uses the existing daemon/protocol transport.
- Review the Rust integration tests as the reproducible manual equivalent and inspect a temporary Markdown mirror after rename propagation.
- Run:
  - `cargo run --manifest-path src-tauri/Cargo.toml -p bridge-protocol --bin generate-protocol-artifacts`
  - `cargo test --manifest-path src-tauri/Cargo.toml`
  - `bun run check`
  - `bun run test`
- Review the complete diff against every acceptance criterion in #879 before opening the PR.
