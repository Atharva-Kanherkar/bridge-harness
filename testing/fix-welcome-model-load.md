# fix/welcome-model-load: Test Contract

## Functional Behavior

Opening a new chat must never paint a guess and then correct itself.

- A provider CLI's `--version` probe runs once per binary file version, not once
  per adapter descriptor. Health reads, new-chat resolution and model-profile
  saves reuse the recorded answer. A replaced payload (a provider update) has a
  new fingerprint and is re-probed, so an in-place upgrade is still reported.
- The draft-settings resolver keeps the last resolution for the same project
  while it re-resolves new choices, so a model or worktree pick does not blank
  the pill or flip a toggle back mid-request. A change of project still clears.
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
- The host's resolution of a draft that carries no explicit model override is
  stored as the next chat's starting choice, together with its catalog label.
- An explicit pick in the Welcome picker is a one-off override for that draft and
  is never stored as the next chat's default. The next chat must not promise a
  model the host would not create, which is the same class of bug as naming a
  fallback it is about to drop.
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
  - `TestReadLastDraftModel_KeepsUnknownHarnessForTheResolver`
  - `TestWriteLastDraftModel_NoStorageDoesNotThrow`
- `src/components/ChatModelControl.test.tsx`
  - `TestChatModelControl_ShowsLoadingStateAfterGracePeriod`
  - `TestChatModelControl_NoLoadingFlashBeforeGracePeriod`
  - `TestChatModelControl_ShowsKnownLabelInsteadOfLoading`
  - `TestChatModelControl_LoadingPickerHasNoFakeDefaultRow`
- `src/useChatDefaults.test.ts`
  - `TestUseResolvedChatDraft_KeepsTheLastResolutionForTheSameProject`
  - `TestUseResolvedChatDraft_ClearsWhenTheProjectChanges`
- `src-tauri/bridge-core/src/binary.rs`
  - `version_probe_runs_once_per_binary_file`
  - `version_probe_rereads_a_replaced_binary`

## Integration / Functional Tests

- `src/App.test.tsx`
  - A stored new-chat choice paints the Welcome pill on first mount while every
    adapter catalog is empty, and the pill never reads `Default` or a raw id.
  - A resolved draft with no override is stored as the next chat's choice.
  - A one-off pick on the Welcome surface is not stored as the next chat's
    default.
  - With no stored choice and empty catalogs, the Welcome pill becomes
    `Loading models…` after the grace period and settles to the catalog label.

## Smoke Tests

- `bun run build` succeeds.
- `bun run test` succeeds.

## E2E Tests

- N/A. The flow is covered by the App-level jsdom integration tests above;
  there is no packaged-app harness for the Welcome surface.

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
