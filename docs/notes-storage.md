# Notes storage and navigation

Notes live as Markdown files under the configured notes root. `notes.db` stores
Yjs update history and rebuildable indexes; every person save, agent write, and
file import uses the same Notes service write path.

Schema v3 adds `pages_fts` (FTS5 with the `porter unicode61` tokenizer) and
`page_links`. Migration from v1 or v2 backfills live pages immediately.
`rebuild_index` replays the update log and reconstructs all three page indexes.

Search quotes each alphanumeric token and applies prefix matching with AND.
Results rank title matches above body matches, with tags also indexed. FTS5's
two unindexed identity columns receive zero BM25 weight, followed by title,
body, and tag weights of 5, 1, and 2. Stemming follows SQLite's Porter rules;
it does not equate every word with a similar meaning. A punctuation-only query
uses an escaped literal substring search.

Links use `[[Page title]]`, `[[Page title|Alias]]`, or
`<page url="bridge://p/<id>">Title</page>`. Inline and fenced code are excluded.
Wiki titles resolve case-insensitively within the same space, including the
first-line display title of untitled notes. Duplicate titles choose the oldest
live page, with its ID breaking ties. Unresolved links resolve when their target
arrives; deleted sources disappear from backlinks. Each backlink includes its
source page, kind, quoting line (at most 200 characters), and update timestamp.

Renaming a titled page rewrites incoming wiki links and matching child page tag
labels, preserving aliases and bytes outside the links. Each affected page gets
an attributed history update and a Markdown mirror through the normal write
path. Propagation commits each page separately.

`notes/update_note` accepts `parentId` or `clearParent: true`. Moving or promoting
an Inbox note opens it unless an explicit state was supplied. Parents must be
live pages in the same space and cannot be tasks, the page itself, or its
descendants. Validation shares the write transaction, rejects malformed cycles,
and examines at most 64 ancestors.

The editor link picker, backlinks panel, and nested sidebar tree consume these
backend capabilities in the frontend phase.
