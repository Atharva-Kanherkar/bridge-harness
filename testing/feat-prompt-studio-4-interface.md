# feat/prompt-studio-4-interface — Test Contract

Builds the Prompt Studio interface: the full editing surface over the typed
prompt API shipped in feat/prompt-studio-3-prompt-protocol (#284). Implements
#242 (Prompt Studio 4/8); parent #238.

## Functional Behavior

### Mount point

- A new `SettingsScreen` section id `"prompts"` ("Prompt Studio") joins the
  existing `Section` union and its left-nav. The section renders the new
  `src/components/PromptStudio.tsx` component full-height inside the settings
  content pane. No routing or sidebar changes are required.

### Target and section navigation

- The left rail lists every `PromptTargetChoice`: Orchestrator, six worker
  roles (Research, Implementation, Verification, Planning, Documentation),
  and Direct session. Selecting a target loads its stack via
  `bridgeApi.promptStack(target)`.
- Each section row shows: the section id, a live token estimate
  (`tokenEstimate`), and a "modified" badge when `state !== "default"`
  (a distinct "deleted" treatment when `state === "deleted"`). The selected
  section is visually marked and reachable by keyboard.
- `direct_session` has no Bridge-stable sections; the rail shows an explicit,
  honest empty state (nothing to edit), not a blank panel.

### Editor

- Section editing reuses `CodeEditor` (CodeMirror 6) with markdown language
  support (`path="prompt.md"`); `docKey` is `${target}:${sectionId}` so
  switching sections re-seeds cleanly. No second editor implementation.
- Dirty state: the editor starts from the section's effective text (default
  text when `default`; disabled/read-only presentation when `deleted` until
  reset). Any unsaved change marks the section and the surface dirty. Navigating
  away from a dirty section keeps the draft per section for the session (drafts
  are keyed like `docKey`) and does not silently discard or auto-save.
- Save: the Save action calls `savePromptSection(target, sectionId, draftText)`
  and adopts the returned `{ revision, stack }`. `⌘S`/`Ctrl+S` inside the
  editor triggers the same save (CodeEditor's Mod-s wiring). Saving is
  reflected in the section row (badge, token estimate) without a manual reload.

### Lint before save

- While a draft would drop a required marker for its section (same vocabulary
  the compiler/test suite requires, e.g. `bridge-delegate` for
  `delegation_protocol`), a visible warning renders **before** save, naming the
  missing marker and the consequence. Warnings warn-explain-allow: Save stays
  enabled. Stack-served `lintWarnings` render in the same region.

### Reset

- Per-section reset calls `resetPromptSection(target, sectionId)` and adopts
  the returned stack; the editor re-seeds to the default text. Enabled only
  when the section is overridden or deleted.
- Whole-target reset resets every non-default section of the selected target
  (one `resetPromptSection` call each), behind a confirmation. It must leave
  other targets untouched.

### Revision history and restore

- Each section exposes its bounded revision history (`revisions`): operation,
  relative/absolute time, and restore action per row. Restore calls
  `restorePromptRevision(target, sectionId, revisionId)` and adopts the result;
  the editor re-seeds to the restored text. History comes from the stack views
  (bounded window is acceptable; no separate fetch).

### Compiled preview and honesty rules

- A preview panel loads `previewCompiledPrompt(target)` and splits:
  - **Exact Bridge bytes**: `stablePrefix` and `variableSuffix` shown verbatim,
    labeled exact, with `prefixHash`, `prefixId`, `prefixBytes`,
    `prefixTokenEstimate`.
  - **Provider layers**: `providerLayers` render separately from the exact
    bytes, each labeled by its `source`. `unavailable` rows show the adapter
    detail/reason; nothing provider-owned is ever presented as exact bytes.
- Cache impact: the panel shows whether the saved Bridge prefix changed
  (compare current `prefixHash` against the previously loaded one) as a
  "prefix changed → next turn rebuilds cache" indicator. Any provider-side
  cache effect is labeled **estimated** unless a future telemetry-backed source
  says otherwise; the UI copy must not claim measured provider cache savings.

### Import / export

- Export writes one JSON file describing the current overrides for the whole
  app keyed by target and section id (state + text), via a browser download.
- Import reads such a file, validates its shape (reject unknown targets/
  sections, malformed entries, non-object roots), applies valid entries through
  `savePromptSection`, reports a per-section summary, and refreshes the stacks.
- Both paths work in browser mode deterministically (tests stub the download
  anchor and file input rather than real OS dialogs).

### Styling and accessibility

- Tailwind v4 utilities and existing tokens/classes only. No standalone
  stylesheet, no CSS-in-JS, no inline `style` for anything a utility can do.
- Every control has an accessible name (nav landmarks, buttons, listboxes);
  the section list is keyboard navigable; save success/failure is announced
  via an aria-live region; ⌘S save behavior is covered by a test.

### Browser-mode mock parity

- All behavior above runs against the existing deterministic mocks in
  `src/api.ts` when not in Tauri. No component test may require the Tauri
  runtime.

## Unit Tests (`src/components/PromptStudio.test.tsx`)

Vitest + jsdom, `createRoot`/`act` style consistent with neighboring tests.

- `renders_target_specific_stacks` — orchestrator lists `bridge_role` +
  `delegation_protocol`; a worker target lists `worker_contract`;
  `direct_session` shows the empty state.
- `section_rows_show_token_estimates_and_modified_badges` — after an override,
  the badge appears and the token estimate reflects the edited text.
- `editing_marks_dirty_and_save_persists_via_api` — typing dirties the draft;
  save calls `savePromptSection` with the typed text; the returned stack is
  adopted (badge/estimate/history update without reload).
- `keyboard_save_saves_the_draft` — dispatching Mod-s in the editor triggers
  the same save path.
- `per_section_reset_restores_default` — reset calls `resetPromptSection`,
  re-seeds the editor, clears the modified badge.
- `whole_target_reset_resets_only_this_target` — resets each non-default
  section of the target and leaves other targets' state untouched.
- `lint_warning_appears_before_save` — a `delegation_protocol` draft without
  `bridge-delegate` shows the warning while remaining savable.
- `revision_history_lists_operations_and_restore_works` — history rows render;
  restoring calls `restorePromptRevision` with the right ids and re-seeds.
- `preview_splits_exact_envelopes_from_provider_layers` — exact labels on
  stablePrefix/variableSuffix with hash/id/bytes; provider rows labeled
  unavailable with their detail, never presented as exact.
- `cache_impact_tracks_prefix_hash_changes` — after a save that changes the
  prefix hash, the changed-prefix indicator appears; provider cache wording is
  labeled estimated.
- `import_rejects_malformed_files_and_applies_valid_entries` — bad shapes are
  rejected without mutating anything; a valid file saves its entries and
  refreshes stacks.
- `export_downloads_one_json_file` — export produces exactly one download with
  the expected override payload.
- `controls_have_accessible_names_and_live_region_announces_saves`.

## Integration / Functional Tests

- N/A beyond component level — the typed API and mocks are covered by
  `src/api.test.ts` from 3/8. The component must compose only existing API
  methods; any missing capability is out of scope for 4/8.

## Smoke Tests

- `bun run build` passes (tsc + vite, strict unused checks).
- `bun run test` passes (vitest + cargo untouched by this change).
- In `bun run dev` (browser mode): open Settings → Prompt Studio, switch
  targets, edit + save + reset a section, see preview and history update —
  all against mocks, no console errors.

## E2E Tests

N/A — not applicable for this change; component tests plus the smoke pass in
browser mode cover the acceptance criteria.

## Manual / cURL Tests

Not applicable (desktop/browser UI change, no network surface). Reviewers
verify via the smoke flow above.
