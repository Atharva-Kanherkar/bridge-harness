# fix/welcome-model-load — Test Contract

## Functional Behavior

Opening a new chat must never paint a guess and then correct itself.

- A new chat's harness/model is resolved in this order:
  1. an explicit `$harness` override (unchanged),
  2. the current direct session's harness/model (unchanged carry-over),
  3. the last confirmed new-chat choice read from storage (new),
  4. the `standard_orchestrator` model profile (unchanged),
  5. the first available adapter's default model, then its first catalog model (unchanged).
- The stored choice is used only when its adapter exists in the health payload and
  is available. A stored harness that is no longer installed falls through to the
  next source.
- A stored model id is kept while that adapter's catalog is still empty (discovery
  has not landed yet). Once the catalog is non-empty and no longer lists the id,
  the stored entry is dropped and resolution continues from the profile, so a
  retired model is never sent to a provider.
- Every draft creation (`openNewChat`, `openWorkspaceDraft`) and every submit
  stores the harness/model it used, together with the catalog label when one is
  known. An explicit pick in the Welcome picker stores immediately.
- `ChatModelControl` shows a loading state when the selected adapter has no
  catalog yet **and** no label is known for the selected model:
  - the pill reads `Loading models…`, carries a spinner, and sets
    `aria-busy="true"`;
  - the state appears only after a 600 ms grace period, so a fast load never
    flashes;
  - the picker, if opened during that window, shows a single loading row instead
    of a fabricated `Default` option.
- A label from a previous catalog read (the stored choice) is shown immediately
  instead of the loading state, and is replaced by the live catalog label once
  discovery lands.
- Nothing changes when catalogs are already loaded: the pill and picker render
  exactly as before.

## Unit Tests

- `src/newChatModel.test.ts`
  - `TestResolveDraftHarnessModel_PrefersExplicitOverride`
  - `TestResolveDraftHarnessModel_CarriesCurrentDirectSession`
  - `TestResolveDraftHarnessModel_UsesStoredChoiceOverProfile`
  - `TestResolveDraftHarnessModel_IgnoresStoredChoiceForUnavailableAdapter`
  - `TestResolveDraftHarnessModel_KeepsStoredModelWhileCatalogEmpty`
  - `TestResolveDraftHarnessModel_DropsRetiredStoredModel`
  - `TestResolveDraftHarnessModel_FallsBackToProfileThenAdapter`
- `src/lastDraftModel.test.ts`
  - `TestReadLastDraftModel_RoundTrips`
  - `TestReadLastDraftModel_IgnoresMalformedPayload`
  - `TestReadLastDraftModel_IgnoresUnknownHarness`
  - `TestWriteLastDraftModel_NoStorageDoesNotThrow`
- `src/components/ChatModelControl.test.tsx`
  - `TestChatModelControl_ShowsLoadingStateAfterGracePeriod`
  - `TestChatModelControl_NoLoadingFlashBeforeGracePeriod`
  - `TestChatModelControl_ShowsKnownLabelInsteadOfLoading`
  - `TestChatModelControl_LoadingPickerHasNoFakeDefaultRow`

## Integration / Functional Tests

- `src/App.test.tsx`
  - A stored new-chat choice paints the Welcome pill on first mount while every
    adapter catalog is empty, and the pill never reads `Default` or a raw id.
  - Picking a model on the Welcome surface writes the choice to storage.
  - With no stored choice and empty catalogs, the Welcome pill becomes
    `Loading models…` after the grace period and settles to the catalog label.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds.

## E2E Tests

- N/A — the flow is covered by the App-level jsdom integration tests above; there
  is no packaged-app harness for the Welcome surface.

## Manual Tests

1. `bun run dev`, open the app, press the new-chat shortcut twice with a 2 s gap.
   The model pill must read the same model both times and must not change text
   once the picker settles.
2. Clear `bridge.chat.lastDraftModel` from storage, reload, open a new chat. The
   pill must read `Loading models…` with a spinner, then the real model label.
3. Open the picker during that window: it must show `Loading models…`, not a
   `Default` row.
4. Sign out / remove an adapter whose model was stored, then open a new chat: the
   pill must fall back to another available adapter, never the removed one.
