# taste-ledger-fields — Test Contract

Sub-issue 1 of the taste layer epic (#1196): #1197. Taste rules will live on
the memory ledger beside memories, so the ledger needs a `layer`, a
repository scope, links to the signals behind a record, and an outcome
score. Nothing writes taste records yet; this change only makes room for
them and guarantees that no memory surface ever sees one.

## Functional Behavior

1. **Schema 105.** `memory_records` gains `layer` (`memory` | `taste`, default
   `memory`), `path_globs` (JSON array, nullable), `outcome_score` (REAL,
   default 0), `last_confirmed_at` (nullable) and `visibility` (`private`
   only, default `private`). New table `memory_evidence(record_id,
   signal_id, weight, added_at)`, one link per pair. Existing rows read
   `memory`, `private`, 0, and nothing else about them changes.
2. **Project scope.** `project:<owner>/<repo>` parses, lowercased. An owner or
   repo is GitHub-shaped (letters, digits, `-`, `_`, `.`, never `.` or `..`);
   anything else is refused. `account:local` and `workspace:<id>` still parse.
3. **Memory surfaces read the memory layer only.** List, list as of, search,
   the packet, extraction's known and forgotten bodies and reconcile set,
   near-duplicate and same-text checks, dreaming, the scope budget, the
   proposal allowance, retiring unused memories, insights, activity, the
   Memory app badge and external import ignore taste records, in every
   scope.
4. **The taste layer lists by itself.** `memory/list_memory_records` accepts
   `layer` (`memory` default, or `taste`). A page cursor names its layer, and
   a cursor from one layer is refused by the other.
5. **A successor inherits.** An edit (supersede), an update proposal, a merge
   of proposals, a dream undo copy and a restore carry their sources' layer,
   path globs (the union; none if any source has none), highest outcome
   score, latest confirmation, visibility, and the union of their evidence.
   Merging a memory with a taste rule is refused and changes nothing.
6. **Wire.** `MemoryRecord` gains `layer`, `pathGlobs`, `outcomeScore` and
   `evidenceCount`, each omitted at its default, so a memory serializes
   byte-for-byte as before and a record without them reads as a memory.

## Unit Tests

**Rust (`bridge-core/src/memory_taste_fields_tests.rs`, new)**
- `every_record_written_without_a_layer_is_a_private_memory`
- `the_schema_refuses_an_unknown_layer_or_a_shared_visibility`
- `the_migration_fills_existing_rows_and_can_run_twice`
- `project_scopes_name_one_repository`
- `layers_parse_and_nothing_else_does`
- `no_memory_surface_sees_a_taste_rule` (packet text byte-identical before
  and after rules arrive)
- `the_taste_layer_lists_by_itself_and_its_cursor_stays_in_it`
- `an_edit_keeps_a_rule_a_rule_with_its_evidence_paths_and_trust`
- `a_merge_of_rules_keeps_every_link_the_widest_paths_and_the_best_trust`
- `a_memory_and_a_rule_never_merge`
- `an_edited_memory_stays_a_memory`

**Rust (`bridge-protocol/src/messages/memory.rs`)**
- `a_memory_reads_and_writes_as_it_did_before_layers`
- `a_taste_rule_carries_its_layer_paths_trust_and_evidence`

## Verification
`cargo test -p bridge-protocol` (drift gate) and `cargo test -p bridge-core
--lib` for `memory`, `store::`, `apps::`, `import` and `external_import`
green; protocol artifacts regenerated; migration 105 run on a copy of a real
ledger leaves every row's id, status and body unchanged.
