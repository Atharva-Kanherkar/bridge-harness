# feat/apps-pin-controls: Test Contract

Issue #1015. Pin and unpin apps from the launcher grid and the sidebar. Plan
note: `bridge://p/01a1dfed-6eea-7831-b745-09cca5240378`. Frontend only; core,
protocol and the 3-pin limit are unchanged.

## Functional Behavior

### Launcher tile

- Every app tile has a pin button in its top-right corner, named
  "Pin <Name> to the sidebar" or, on a pinned tile, "Unpin <Name>".
- Visible when the tile is hovered or active, and always on a pinned tile
  (it replaces the static pin mark).
- Clicking it runs the same path as ⌘P: pins or unpins through core, and the
  footer's live region says "Pinned <Name> to the sidebar.", "Unpinned
  <Name>." or core's refusal ("You can pin 3 apps. Unpin one first.").
- Clicking it never opens the app and never closes the launcher. Pressing the
  mouse on it does not move focus out of the search input.
- It is not a tab stop (`tabIndex=-1`): the tile stays the listbox option and
  ⌘P stays the keyboard path.

### Sidebar

- Each pinned app row has an "Unpin <Name>" button, a sibling of the row
  button, revealed on row hover and on keyboard focus.
- Clicking it unpins through core and does not open the app.
- The Apps row has no Unpin button.

## Unit Tests

Vitest (jsdom):

- `AppLauncher.test.tsx`: the tile pin calls `onTogglePin` with the id, the
  launcher stays open and `onOpenApp` is not called; a pinned tile's button
  is named "Unpin <Name>"; a refusal shows in the footer.
- `BridgeSidebar.test.tsx`: the Unpin button calls `onUnpinApp` with the id
  and not `onOpenApp`; the Apps row has none.

## Integration / Functional Tests

- `App.test.tsx` (mock mode): pinning from the launcher tile adds the app to
  the sidebar; Unpin in the sidebar removes it.

## Smoke Tests

- `bun run build` and `bun run test` (vitest) green; `tsc -b` clean.

## E2E Tests

- Mock mode in a browser: hover a tile, click its pin, see the sidebar row;
  hover the row, click Unpin, see it gone; pin a 4th app and read the refusal.
  Dark and light screenshots.

## Manual / cURL Tests

N/A: no wire change.
