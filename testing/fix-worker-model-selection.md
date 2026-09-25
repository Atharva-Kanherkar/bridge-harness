# fix/worker-model-selection — Test Contract

The report: in Settings › Models › Workers, the model picker is greyed out, so a
worker role's model cannot be changed (issue #708).

## Functional Behavior

- Every worker and verification role ships in `track_standard` selection mode
  (`model_profiles.rs::recommended_profiles`, `recommendedProfileDrafts`). The
  "Provider and model" control was disabled in that mode, so the only way to
  change a worker model was to first find the separate "Selection behavior"
  control and flip it to "Pinned model".
- **The model picker is live in both modes.** A role's model is chosen in the one
  control that names models, whether the role is tracking its tier's standard
  default or holding a pinned model.
- **Choosing a model pins the role.** The pick writes
  `{ provider, model, selectionMode: "pinned", pinned: true, learningEnabled: false }`
  in the one save the page already makes, exactly like the orchestrator's picker
  and exactly like the "Selection behavior → Pinned model" transition. No second
  click, no second control to discover first. The row's pill flips to "Pinned".
- **"Selection behavior" remains the way back to tracking.** Setting it to "Track
  standard" writes `{ selectionMode: "track_standard", pinned: false,
  learningEnabled: <unchanged> }` and the "Allow learning" switch becomes live
  again. The stored model stays as the last pinned value; the resolver stops
  reading it while tracking.
- **Nothing is a dead control.** The picker states, in prose, why a tracked role
  follows a standard and what choosing a model does. The orchestrator's picker
  and worker pickers carry the same shape of behavior.
- A pick only fires on an actual change: re-choosing the model already shown
  emits no `onValueChange`, so browsing the list and picking the current value
  cannot silently pin a role the user was only inspecting.
- The one remaining disable is `busy`, the in-flight save, which still applies to
  every control on the page.

## Surfaces in scope

- `src/components/settings/ModelsPage.tsx` — Settings › Models, worker and
  verification rows.
- `src/components/ModelProfileEditor.tsx` — the same role profiles in the router
  settings dialog and the onboarding wizard, which carried the identical lock.

## Unit Tests

- `ModelsPage.test.tsx` — a tracking worker row's "Provider and model" trigger is
  not disabled; choosing a different catalog model sends one save carrying
  `selectionMode: "pinned"`, `pinned: true`, `learningEnabled: false`, and the
  new provider and model; the untouched roles are byte-identical in that payload.
- `ModelsPage.test.tsx` — setting "Selection behavior" back to "Track standard"
  sends `selectionMode: "track_standard"` and `pinned: false`, and "Allow
  learning" is enabled again.
- `ModelsPage.test.tsx` — a pinned row's picker is enabled too, so a pinned model
  can be changed directly.
- `ModelsPage.test.tsx` — the "Provider and model" row explains the tracking
  default and the pinning consequence.
- `ModelProfileEditor.test.tsx` — a tracking worker role's "Provider & model"
  field is not disabled, and choosing a model pins the role in one `onChange`
  with the same four fields. Choosing a model in the orchestrator's field keeps
  its current behavior.

## Integration / Functional Tests

- The full existing `ModelsPage`, `SettingsScreen`, `AdaptiveSetup`, and
  `modelProfiles` suites stay green: the page still renders no native
  `select` and no native `input[type=checkbox]`, still saves the whole profile
  set per change, still carries no Save button, and the orchestrator still gets a
  tier-free picker with no "Selection behavior" row.
- No Rust change. `model_profiles::resolve_profile` already honors a pinned
  worker role by exact model id, and `pinned_profile_survives_standard_promotion`
  and `tracking_profile_follows_new_standard_without_persisted_rewrite` remain
  the authority for what each mode means at launch.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds.

## E2E Tests

N/A — the fix is a static enable/disable condition on two settings surfaces. The
contract for it is the trigger's `disabled` property and the exact payload of
one save, both asserted in unit tests. No provider session is launched, so no
claim is made about a live worker running the newly pinned model.

## Manual / cURL Tests

- Settings › Models › Workers: expand Implementer. "Provider and model" is
  clickable while the row reads "Tracks standard". Pick a different model. The
  pill reads "Pinned", "Selection behavior" reads "Pinned model", and "Allow
  learning" is greyed out. Reopen Settings: the row still reads the picked model.
- Same row, set "Selection behavior" back to "Track standard". The pill returns
  to "Tracks standard", "Allow learning" is live again, and the picker still
  shows the last picked model.
- Settings › Workers › "Advanced routing and role profiles": the same flip works
  in the router dialog, and Save there persists it.
- Empty catalog: with no available provider the picker stays disabled
  (`options.length === 0` in the shared `Select`), which is correct and
  unchanged.

## Out of scope

- The reasoning-effort control for a worker role still offers all four levels
  regardless of the chosen model's advertised levels, and a pick does not
  normalize the effort. That gap is reachable today through the effort control
  alone, so this change neither introduces nor closes it.
- A tracked role's stored `model` is a snapshot of its tier default that Rust
  refreshes only when the model goes unavailable, while the resolver re-derives
  the live tier default at launch. The row therefore shows the stored snapshot.
  Making the row show the resolved model is a display change to the same control
  and is deliberately not part of this fix.
- `PresetsPage`'s "Model" is disabled while a preset's harness is "Bridge
  chooses". That is a different control with a different reason (the model is
  meaningless without a harness) and stays as it is.
- No change to tier routing, the learning router, quota gates, or the composer
  model switcher.
