# feat/293-state-decomposition - Test Contract

## Functional Behavior

- The application root provides one shared TanStack Query client to the production app.
- `health` and `modelSetup` load through typed queries during application startup.
- Health-changing backend events invalidate the health query so active UI updates without a restart.
- Successful model setup changes update the shared model-setup cache immediately.
- Opening Work fetches `workBoard` through TanStack Query without selecting, creating, or starting a session.
- Reopening Work and actions that mutate Work data refetch the board through the query cache.
- A failed first Work read shows a fatal board error; a failed refetch preserves the cached board and shows a separate refresh error.
- Concurrent or superseded Work reads cannot overwrite newer cached data with stale results.
- Dialog selection (`workspace`, `orchestrator`, `router`, `memory`, or closed) lives in a Zustand store rather than an `App.tsx` `useState` call.
- Existing dialog open and close behavior remains unchanged.
- `src/api.ts` and the generated protocol registry remain unchanged.
- `App.tsx` has fewer `useState` calls and fewer lines than it did before this change.

## Unit Tests

- `src/queryClient.test.ts` verifies stable query keys and desktop-appropriate query defaults.
- `src/uiStore.test.ts` verifies opening, replacing, and closing the active modal.
- `src/workWiring.test.ts` verifies Work uses TanStack Query refetching, remains lazy, starts nothing, and preserves cached data on refetch failure.
- Source-level wiring tests verify `App.tsx` no longer declares local state for `health`, `modelSetup`, `workBoard`, Work errors, or `modal`.

## Integration / Functional Tests

- Existing App tests mount the app under its query provider and continue to pass.
- Existing Work wiring and WorkView tests continue to pass.
- Model setup completion and settings callbacks write the returned setup into the shared query cache.
- Health adapter and provider-login events invalidate the shared health query.

## Smoke Tests

- `bun run build` completes successfully.
- The full frontend and Rust test suite passes with `bun run test`.
- The production bundle contains the app and lazy Work view without TypeScript errors.

## E2E Tests

N/A - this refactor changes frontend state ownership without adding a new user journey. Existing mounted App tests cover the affected shell behavior.

## Manual / cURL Tests

N/A - Bridge consumes a typed Tauri API rather than an HTTP endpoint. Manual review should open Work twice, trigger a refresh, and open and close each migrated dialog in the desktop app.
