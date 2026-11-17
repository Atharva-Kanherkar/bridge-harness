# Browser page stays inside its pane at any UI zoom

## Functional behavior

- The native browser page fills the dock's viewport host exactly at every UI
  zoom level (Settings > Appearance > Zoom, Cmd+/Cmd-, pinch), not only at 100%.
- The host rect is read in CSS pixels, clamped to the viewport in CSS pixels,
  then converted to window points by the zoom the webview last confirmed.
- Changing the zoom re-sends the layout without waiting for a resize or the
  next poll.
- Collapsed or offscreen panes still hide the page at every zoom.
- The browser commits from the earlier plain-browser PR (Safari user agent on
  macOS, window clamp, wider dock, restyled chrome) are on main.

## Unit tests

- `BrowserPage.test.tsx`: at zoom 1.25 a host at (400, 80, 360x500) CSS px is
  laid out at (500, 100, 450x625); an overflowing host is clamped in CSS px
  before scaling; a zoom change event relayouts with the new scale.
- `zoom.test.ts`: `appliedZoom()` reports the confirmed level and falls back to
  the last confirmed level when an apply fails.
- Every new frontend case fails without the fix.

## Integration and smoke tests

- `bun run build` and `bun run test` green.
- `cargo test -p bridge-deck embedded_browser` green.

## E2E and manual verification

- In `bun run tauri dev` at zoom 125%, open the browser dock on google.com: the
  page fills the pane below the omnibox and the modern Google homepage loads.
- Cmd+0 and Cmd+ while the page is open: the page tracks the pane each time.
