# feat/issue-294-tanstack-router-settings-form — Test Contract

Source: [issue #294](https://github.com/Atharva-Kanherkar/bridge-harness/issues/294) — adopt
`@tanstack/react-form` for `RouterSettingsDialog`'s router preferences, exclusion lists, and
learning-schedule sub-form. Out of scope: `ModelProfileEditor` / role model profiles (its own
state, untouched), the three external scheduler registration cards, and any other
single-field dialog.

## Functional Behavior

- `RouterSettingsDialog`'s router-preference fields (mode, minimum pass probability, pin
  harness, pin model, exclude harnesses, exclude models) and the learning-schedule sub-form
  (enabled, mode, cadence, spend ceiling, token ceiling) are managed through one
  `@tanstack/react-form` `useForm()` instance instead of six-plus separate `useState` calls.
- Picking a harness in "Pin harness" clears "Pin model" via a field `listeners.onChange`
  rule on the `pinnedHarness` field (calling `fieldApi.form.setFieldValue("pinnedModel", null)`),
  not an inline setter side effect spelled out in the JSX `onChange`.
- Minimum pass probability: typing a value outside 0–100 shows a visible validation message
  ("Enter a percentage between 0 and 100.") under the field instead of silently clamping to
  the nearest bound. Save is disabled while the error is present.
- Learning cadence: typing a value below 15 minutes shows a visible validation message
  ("Cadence must be at least 15 minutes.") instead of silently rewriting the typed number to
  15. Save is disabled while the error is present.
- Spend ceiling / token ceiling: a negative value shows "Must be zero or greater." instead of
  being silently clamped to 0.
- Cadence, spend ceiling, and token ceiling cross to the Rust host as `i64` fields
  (`bridge-protocol/src/messages/learning.rs`). A fractional value (e.g. `15.5`) must fail
  validation client-side too ("Cadence must be a whole number of minutes." / "Enter a whole
  number.") and disable Save — it must never reach `updateLearningSchedule` and surface as a
  raw deserialization error instead of an inline message.
- Emptying the pass-probability field and blurring it still reverts the displayed value to
  the last-saved percentage (existing behavior, unchanged — this is a discard-invalid-draft
  action, not a silent clamp of a real number).
- Async load order is unchanged: opening the dialog fetches `routerPreferences`, `modelSetup`,
  and `learningState` together and resets the form once all three resolve; closing the dialog
  drops unsaved schedule edits and shows the loading state (no schedule fields) on reopen.
- The generation-guarded merge that protects an in-flight schedule read from a stale
  `onLearningJobChanged` notification is preserved, now diffing the form's live `schedule`
  field value instead of a `learning.schedule` state slice.
- Save persists preferences, then role-model profiles (if changed), then the learning
  schedule (only if its user-facing fields changed from the last saved schedule) — same
  ordering and same-no-op behavior as before.

## Unit Tests

- `scheduleUserFieldsChanged` — unchanged export, still exhaustively covers
  enabled/cadence/mode/budget/tokens vs. runner-owned `nextRunAt`.
- `evaluatorExecutionLabel` — unchanged export, exhaustive over `evaluationExecution`.

## Integration / Functional Tests (`AdaptiveSetup.integration.test.tsx`, jsdom)

All pre-existing cases must keep passing unmodified:
- runs manual learning and renders its explicit no-op report
- refetches learning state when a learning job changes
- a slower learning-state read cannot win
- an emptied pass floor cannot commit zero on the way to a number
- does not rewrite the learning schedule when save has no schedule edits
- closing the dialog drops unsaved schedule edits
- offers editable evaluator spend and token ceilings
- does not create a profile version when settings save without profile edits

New cases to add:
- typing 150 into the pass-probability field shows "Enter a percentage between 0 and 100."
  and the Save button is disabled until the value is corrected.
- typing 5 into the cadence field shows "Cadence must be at least 15 minutes." and Save is
  disabled until corrected.
- typing 15.5 into the cadence field shows "Cadence must be a whole number of minutes." and
  Save is disabled until corrected.
- typing -1 into the spend-ceiling field shows "Must be zero or greater." and Save is
  disabled until corrected.
- picking a harness in "Pin harness" after a model was pinned clears "Pin model" back to
  "Automatic".

## Smoke Tests

- `renderToStaticMarkup(<RouterSettingsDialog .../>)` (the existing SSR smoke test in
  `AdaptiveSetup.test.tsx`) still renders without throwing and still contains all previously
  asserted strings.

## E2E Tests

N/A — no Tauri/`bridged` round trip is exercised by this change; `bridgeApi` stays mocked in
tests exactly as before.

## Manual / cURL Tests

N/A — pure frontend component change, no wire schema or Rust surface touched.
