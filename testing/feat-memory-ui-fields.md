# feat/memory-ui-fields

The surface catches up to real writers — and renders nothing speculative.
Every visible field is a field something writes.

## Supersede reaches the wire

- `memory/supersede_memory_record` takes recordId, body, and an optional kind;
  kind is inherited from the old record when omitted. Only an active record
  supersedes. Publishes `memory-changed`.
- `MemoryRecord` gains an optional `supersedes` id so an edited pin can say it
  replaced an earlier one. `superseded_by` stays off the wire: the active list
  never contains a superseded row, so it would be a field without a reader.

## Edit in the dialog

- An Edit affordance on a pin row moves its body and kind into the composer;
  saving calls supersede, never save-then-forget and never an in-place update.
  Cancelling editing restores the plain composer and writes nothing.
- The edited row leaves the list and its replacement appears — the refetch
  rides the same `memory-changed` hint as every other write.

## Chips render only what a writer populated

- A row whose provenance is `model_proposal` (an extracted record, whether
  manually approved or automatically promoted) is chipped as extracted; a
  `user_explicit` row carries no provenance chip.
- Confidence renders only when present. An explicit pin shows no confidence —
  no badge at zero applies to metadata too.
- A record carrying `supersedes` says it replaced an earlier pin.

## Out of scope

Chain browsing beyond the replaced-an-earlier-pin marker, a Runbook tab (its
producer is the consolidation slice), workspace scope chips (no workspace
writes exist), TTL and freshness (no producer), prompt injection.
