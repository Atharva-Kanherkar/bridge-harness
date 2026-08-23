# feat/prompt-studio-3-prompt-protocol — Test Contract

Exposes Prompt Studio through typed protocol methods: stack retrieval, section
save/reset, revision history/restore, and exact Bridge-authored compiled
previews. Implements #241 (Prompt Studio 3/8); parent #238.

Revised after independent review: adds atomicity, bounded histories, typed
vocabularies, a shared compile builder, and mock-parity requirements.

## Functional Behavior

- Five new `config/*` methods exist and are wired end to end (registry → params/results → dispatch → core API → Tauri command → generated TS/schemas → frontend wrapper → browser mock):
  - `config/get_prompt_stack` — resolves one target's section stack.
  - `config/save_prompt_section` — persists an override.
  - `config/reset_prompt_section` — restores a section to its built-in default.
  - `config/restore_prompt_revision` — restores a prior revision as a new revision.
  - `config/preview_compiled_prompt` — compiles the exact Bridge-authored envelopes for one target.
- Params carry `target` as a typed enum whose wire values equal the storage keys (`orchestrator`, `worker:research`, `worker:implementation`, `worker:verification`, `worker:planning`, `worker:documentation`, `direct_session`). Unknown targets are rejected at deserialization; unknown target/section combinations are rejected by the same validation as 1/8.
- Worker contracts are depth-sensitive: every method accepts an optional `depth` (default 0, rejected when negative or above `DEFAULT_MAX_DEPTH`). Depth is validated **before any write**: a mutation that fails depth validation leaves the database untouched and publishes nothing.
- Every mutation returns `{ revision, stack }`: the revision it appended plus the fresh stack for that target. Re-running save/reset on identical state appends deterministically (append-only history is never rewritten) and returns equivalent stacks.
- Stack views report, per section: current state (`default` / `overridden {text}` / `deleted`), built-in default text, effective text (null when deleted), byte count (UTF-8) and byte-derived token estimate of the effective text, lint warnings over the effective text, and the most recent bounded window of the append-only revision history.
- Revision history in every view is **bounded**: only the most recent `MAX_REVISIONS_IN_VIEW` (50) revisions are included, oldest-first within the window. Restoring any stored revision id still works regardless of the window.
- Deleted sections are omitted from compiled output but remain visible in the stack view as `deleted`.
- Revision operations and provider-layer sources cross the wire as **serialized enums**, not free strings: `operation` ∈ `override | delete | reset | restore`; source ∈ `reported | measured | estimated | unavailable`.
- The live turn path and the studio preview share one helper that turns a resolved stack into a `PromptCompiler`, so their compositions cannot drift.
- Preview returns, for one target:
  - `stablePrefix` — the exact bytes `<bridge-stable-prompt …>…</bridge-stable-prompt>` produced from the resolved sections alone;
  - `variableSuffix` — the exact `<bridge-variable-context>` envelope with an empty sections list (runtime task/session/restoration/memory content is not fabricatable);
  - `prefixHash`, `prefixId`, `schemaVersion`, `prefixBytes`, `prefixTokenEstimate` matching those bytes;
  - `stack` (the full bounded stack view, so revisions and lint travel with the preview);
  - `providerLayers` — one row per registered adapter's provider-base layer carrying a typed source status, plus optional detail. No adapter can currently read back its provider base, so every row must be `unavailable` with the authority reason attached. Provider-owned text is never fabricated.
- The preview claims exactness only for the two Bridge-authored envelopes; anything else (configured project rules, tool schemas, provider base, native history) is out of its scope and either absent or reported honestly as unavailable.
- All new param structs deny unknown fields; result payload types pass the protocol mirror gate against their core counterparts.
- Browser-mode mocks mirror native semantics used by tests and demos: depth validated before any mock mutation, byte counts computed as UTF-8 lengths, previews hashed with real SHA-256 over the exact envelope bytes, reset-all appends reset revisions instead of erasing history, and unknown-target/foreign-revision requests reject.

## Unit Tests

- `params_round_trip_and_reject_unknown_fields` (messages/config.rs) — each new params struct round-trips camelCase, rejects misspelled fields, rejects snake_case wire names, and rejects unknown targets; `depth` defaults when absent.
- `prompt_studio` module tests (bridge-core):
  - `stack_reports_states_defaults_sizes_lint_and_revisions` — default stack marks every section default; an override/delete flips state and effective text; sizes match effective-text UTF-8 byte length; token estimate is bytes.div_ceil(4); lint warnings appear on edited text that drops required markers; revision history is complete and ordered.
  - `mutations_return_the_appended_revision_and_fresh_stack` — save/reset/restore each return the appended revision id and a stack consistent with the database.
  - `failed_depth_mutations_have_no_side_effects` — save/reset/restore with an out-of-range depth return Err and leave states, texts, and revision counts exactly as before.
  - `revision_views_are_bounded_to_the_most_recent_window` — more mutations than the bound yield exactly the newest N revisions in the view, oldest-first inside the window; restore of an older out-of-window id still succeeds.
  - `restore_rejects_revisions_from_other_sections` — cross-section restore fails without side effects.
  - `depth_bounds_are_validated` — negative depth rejected; depth above `DEFAULT_MAX_DEPTH` rejected; worker stack text differs between depth 0 and depth 1; orchestrator ignores depth but echoes it.
  - `preview_bytes_are_exact_for_the_resolved_stack` — stable prefix equals a hand-built compile of the resolved sections; variable suffix equals the empty-variables envelope byte-for-byte; prefix hash/id/bytes/token estimate match `PromptMetadata` recomputed from those bytes.
  - `preview_reflects_overrides_deletions_and_depth` — editing/deleting a section changes the preview bytes and prefix hash; worker depth changes worker previews.
  - `provider_layers_cover_every_registered_adapter_with_honest_sources` — one row per entry in `provider_base_prompt_authorities()`, typed sources drawn from the closed enum, every current row `unavailable` with a non-empty reason, and no row carries fabricated bytes.
  - `live_and_studio_share_one_compiler_builder` — the live turn path and the preview use the same resolved-stack-to-compiler conversion (single non-public helper exercised from both).
- `protocol_mirror.rs` — `prompt_studio_payloads_mirror_core`: asserts `PromptStackView`, `PromptSectionView`, `PromptRevisionView`, `PromptSectionMutationResult`, `CompiledPromptPreviewResult`, `PromptProviderLayerStatus`, the section-state enum, the revision-operation enum, and the layer-source enum mirror their bridge-core counterparts document-for-document.
- Existing gates must stay green unchanged: registry totality, params naming, strictness (`additionalProperties: false`), result typing, `generate_handler!` 1:1 parity, shell command-signature parity, API-delegation, committed-artifact drift, orphaned schema files.

## Integration / Functional Tests

- Dispatch round-trip: daemon-side decode → api → serialize produces documents the wire result schema accepts (covered by mirror + gates).
- Frontend boundary test (`src/api.boundary.test.ts`) accepts the five new `call()` methods because they are present in generated `BRIDGE_METHODS`.

## Smoke Tests

- `cargo test -p bridge-core prompt_studio` passes.
- `cargo test -p bridge-protocol && cargo test -p bridged` pass.
- Regenerated artifacts are committed and drift tests pass: `cargo run --manifest-path src-tauri/Cargo.toml -p bridge-protocol --bin generate-protocol-artifacts`.
- `bun run check`, `bun run build`, `bun run test` pass from the checkout root.

## E2E Tests

N/A — this slice adds no UI screen; the Prompt Studio editor belongs to #242. Browser-mode mocks are exercised by `src/api.test.ts`.

## Manual / cURL Tests

N/A — no network API is introduced. In the running dev app, the wrappers can be exercised from the browser console in browser mode via the mocked API object (mock stack reflects saves/resets/restores without persistence).
