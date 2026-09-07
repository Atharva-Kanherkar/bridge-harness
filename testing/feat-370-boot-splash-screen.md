# feat/370-boot-splash-screen — Test Contract

Issue #370. Bridge can show a blank/empty window for a moment while the webview
boots and React hydrates. This adds a branded boot splash — painted before any
JS executes — that covers exactly that gap and gets out of the way the instant
the real UI has painted.

Wordmark only ("BRIDGE"), no borrowed imagery — the reference screenshot in the
issue is a style cue (blurred ground, centered mark, soft fade-in), not an asset
to copy.

## Functional Behavior

- The splash markup lives in `index.html`, outside the React tree, so it is part
  of the very first painted frame — it does not wait on any JS bundle.
- It shows the wordmark `BRIDGE` with a soft opacity/blur/scale entrance
  animation, over a background that already matches the resolved light/dark
  theme (reuses the existing inline theme-resolution script — no flash of the
  wrong ground).
- It is dismissed as soon as React's first paint of the mounted tree has
  happened (two animation frames after `root.render`), independent of whether
  backend data (`reload()`) has finished — this covers the webview-boot +
  hydration flash described in the issue, not the app's own data-loading state,
  which already has its own empty/loading treatment.
- Dismissal fades the splash out and then removes its DOM node entirely — it
  never lingers as a stray hidden overlay.
- `pointer-events: none` for its whole lifetime, so it never intercepts a click
  even mid-fade.
- Reduced motion is handled by the existing global
  `@media (prefers-reduced-motion: reduce)` rule in `src/index.css` (collapses
  animation/transition duration), the same mechanism every other CSS-only
  animation in the app relies on — no bespoke JS branch.
- Dismissal runs exactly once per launch, on both the default `App` render path
  and the `?preview=right-rail` preview path in `src/main.tsx`.
- If `#boot-splash` is missing from the DOM (e.g. a test harness that doesn't
  load the real `index.html`), dismissal is a silent no-op — never throws.

## Unit Tests

`src/bootSplash.test.ts` (`dismissBootSplash`):

- `DismissBootSplash_NoElement` — does not throw when `#boot-splash` is absent.
- `DismissBootSplash_AddsHideClass` — adds the fade/hide class to `#boot-splash`
  once the scheduled animation frames have run.
- `DismissBootSplash_RemovesOnTransitionEnd` — removes the element from the DOM
  after a simulated `transitionend` event.
- `DismissBootSplash_RemovesOnTimeoutFallback` — removes the element after a
  fallback timeout even if `transitionend` never fires (covers environments
  that don't dispatch it, and reduced-motion collapsing the transition faster
  than a listener can attach).
- `DismissBootSplash_Idempotent` — calling it twice does not throw or double-run
  the removal.

## Integration / Functional Tests

N/A — `src/main.tsx` is a 20-line bootstrap entry with no existing test file;
its one new obligation (call `dismissBootSplash()` after each render branch) is
covered by manual verification below rather than a new test harness for a file
that has never had one.

## Smoke Tests

- `bun run build` succeeds — `index.html` stays valid and every Tailwind class
  used in the static splash markup compiles.
- `bun run test` is green, including the new `bootSplash.test.ts`.

## E2E Tests

N/A — no Playwright/full-app-launch harness exists in this repo. Covered by the
manual step below plus the unit tests around dismissal timing.

## Manual / cURL Tests

No backend/API surface changes. Manual verification:

1. `bun run tauri dev` (or a built bundle) and watch the window appear — the
   `BRIDGE` wordmark splash should be visible immediately, fade in briefly, then
   fade out into the normal workspace UI once it has painted.
2. Toggle system dark/light mode and relaunch — splash background should match,
   never flashing the wrong ground.
3. Enable "Reduce motion" in macOS Accessibility settings and relaunch — splash
   should appear/disappear near-instantly, no animation.
