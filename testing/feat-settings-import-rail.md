# feat/settings-import-rail: Test Contract

Import gets its own destination in the Settings rail instead of sitting two levels down under Data & storage.

## Functional Behavior

- The Settings rail lists five destinations in this order: General, Coding agents, Permissions, Data & storage, Import.
- Clicking Import in the rail opens the Move to Bridge page directly, with the rail item marked as the current page.
- The Import page has no "Back to Data & storage" link, because it is a destination of its own.
- Data & storage no longer lists an Import history row. Archived chats stays there.
- The narrow-window section picker lists Import too.
- Settings search still finds the import page.

## Unit Tests

- `sections.test.ts`: the rail order is the five destinations above, `primarySection("import")` is `"import"`, and every section still maps to a rail destination.

## Integration / Functional Tests

- `SettingsScreen.test.tsx`: the rendered rail shows all five labels; clicking Import marks it current and renders the import page; Data & storage renders Archived chats and no Import history row.

## Smoke Tests

- `bun run build`
- `bun x vitest run src/components/settings src/components/SettingsScreen.test.tsx`

## E2E Tests

N/A: a navigation change covered by the mounted component tests.

## Manual Tests

- Open Settings, click Import in the rail, and see Move to Bridge with its Found on this Mac list.
- Open Data & storage and see Archived chats without an Import history row.
