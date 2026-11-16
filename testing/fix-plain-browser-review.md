# Plain browser review fixes

## Functional behavior

- Native layout intersects the requested host rectangle with the current window.
  Right/bottom overflow never moves the page left/up over app chrome. Completely
  offscreen and subpixel rectangles hide the native page.
- Only macOS WKWebView receives the Safari user agent. Other engines retain their
  native identity.
- CI recovers from cached directories occupying Tauri sidecar output paths,
  while preserving the dependency cache and actual sidecar files.

## Unit tests

- `embedded_browser::tests`: unchanged in-window bounds, partial overflow on each
  edge, complete exclusion on each edge, collapsed/subpixel window and host sizes.
- `BrowserPage.test.tsx`: matching frontend intersection and hidden bounds cases.
- Regression fixture for the CI cache repair: replace only directory-shaped
  sidecar outputs, preserve unrelated artifacts, regular files, and source trees.

## Integration and smoke tests

- Run `bun run build` and `bun run test`, including release, sidecar, frontend,
  and Rust workspace suites. Disable Node's experimental native web storage for
  jsdom on local Node versions that expose it.
- Run `cargo check --workspace`.
- Push the reviewed commits to the existing PR branch and verify the new CI jobs.

## E2E and manual verification

Native desktop screenshots are not automated by this review. Existing images in
the PR are illustrative HTML renders, not proof of native behavior. In a desktop
build, open a page and shrink the window while expanded: page pixels must remain
below the toolbar. Open a dialog and confirm the native page hides.
