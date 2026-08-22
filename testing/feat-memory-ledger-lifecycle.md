# feat/memory-ledger-lifecycle

Only the schema extraction and packets cannot land without: statuses,
supersession, and a search index. No column ships without a producer in the
same slice; the parked #214 fields (TTL, confidence, sensitivity, consent,
retention, path scopes, audits) stay parked until something writes them.

## Schema 32

- `memory_records` gains `supersedes` and `superseded_by`, both nullable ids.
- `memory_record_fts` (FTS5, unicode61) indexes active record bodies, kept in
  sync by triggers. Tombstone, reject, and supersede all purge the index row.
- No new tables besides the index. `memory_retrieval_audits` lands with its
  writer in the packet slice, not here.

## Status machine

- Vocabulary grows to `active | proposed | rejected | superseded | deleted`.
- `proposed -> active` happens only through `approve`. `proposed -> rejected`
  through `reject`; rejected is terminal short of a fresh proposal.
- Nothing in this slice produces a `proposed` row; the extractor slice does.
  The machine ships here because the extractor cannot land without it.
- `active -> superseded` happens only through `supersede`, which writes a new
  active record carrying `supersedes` and stamps the old row's
  `superseded_by`. Edit is that, never an UPDATE of a body in place.
- Superseded and rejected rows leave `list` and the index but remain readable
  history; the chain is walkable in both directions.

## Search

- `memory_ledger::search(db, scope_key, query, limit)` mirrors session
  recall: phrase-quoted tokens so user text cannot be FTS syntax, scope bound
  in SQL, only `active` rows, same limit bounds as list.

## Scope decision

Account pins apply everywhere. Explicit save keeps its no-scope wire shape and
its unforgeability test; a workspace-scoped write lands only when a producer
needs it, as its own deliberate wire change.

## Out of scope

Approve/reject/supersede protocol methods and UI (the queue slice and the UI
catch-up slice own those), TTL and stale, confidence, sensitivity, consent,
retention, import/export, retrieval audits, prompt injection, extraction.
