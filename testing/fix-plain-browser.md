# fix-plain-browser — Test Contract

The plain browser pane (`SimpleBrowser` + `BrowserPage` + the `embedded-browser`
shell plugin) is cramped, paints websites in a degraded legacy style, lets the
native page paint over the app chrome, and its controls do not look like the
rest of Bridge. This contract defines "done" for the overhaul PR.

## Functional Behavior

1. **The native page never paints over app chrome or the conversation.**
   The child webview's frame must exactly equal the visible rect of its
   `data-browser-viewport` host, clamped to the app window. A stale or
   out-of-window rect must never be applied as-is: layout bounds are clamped
   to `window.innerWidth/innerHeight` on the frontend, and the Rust side
   refuses bounds wider or taller than the window.
2. **Websites render as they do in Safari.**
   The child webview sends a user agent WebKit-derived sites recognize
   (`Mozilla/5.0 (Macintosh; …) AppleWebKit/605.1.15 (KHTML, like Gecko)
   Version/N Safari/605.1.15`). Google no longer serves its basic-HTML
   fallback; UA-gated sites serve their modern layouts.
3. **The pane fits the screen.**
   - `MAX_DOCK_WIDTH` raised 760 → 1080 so the dock can grow on large windows.
   - The dock's existing Expand control (⌥⌘↩) fills the window with the
     browser; the native webview tracks it (behavior 1 applies in every dock
     mode: split, sheet overlay, expanded, fullscreen conceal).
4. **The browser chrome matches Bridge's Graphite & Paper system.**
   - Token-driven only: `bg-background`, `border-border`, `bg-muted`,
     `bg-accent`, `bg-selection`/`text-selection-foreground`,
     `text-muted-foreground`, `text-destructive`, radius ladder, Geist/Bricolage.
   - Tab strip: pill tabs with hover/active/loading affordances, close buttons
     that appear on hover or when active, roving keyboard focus preserved.
   - Toolbar: back/forward/reload/stop in a bordered control cluster, a
     rounded omnibox field with a focus ring, inspect + open-external at the
     trailing edge; inspecting state reads as selection-colored.
   - Status banners (error / slow / inspecting) are compact, tokened rows.
   - Empty tab, page-error, and selection-review states use the same tokens
     (no hardcoded colors, no ad-hoc CSS files, no inline `style`).

## Unit Tests

- `dockLayout.test.ts` — clamping and persistence against the raised
  `MAX_DOCK_WIDTH` (symbolic; no literal 760 anywhere).
- `SimpleBrowser.test.tsx` — existing suite must stay green (tab mount
  survival, roving tabs, omnibox commit, native-history fallback, snapshot
  races, selection flow). New:
  - `BrowserPage` clamps a host rect that extends past the window (x/width
    within `innerWidth`, y/height within `innerHeight`).
- Rust `embedded_browser.rs` tests:
  - `browser_user_agent_matches_webkit` — the UA string has the
    AppleWebKit/605.1.15, `Version/`, and `Safari/` tokens.
  - `oversized_layout_bounds_are_refused` — a layout command wider/taller
    than the window errors instead of painting over chrome.

## Integration / Functional Tests

- `bun run build` and `bun run test` pass (vitest + cargo test).
- Desktop verification (manual, scripted): the dev app opens
  `https://www.google.com/` and `https://example.com/` in the browser pane;
  screenshots show the modern Google homepage inside the dock with the tab
  strip and omnibox visible above it, no webview pixels over the chat.

## Smoke Tests

- `bun run dev` boots; `SimpleBrowser.test.tsx` and `dockLayout.test.ts` pass.
- `cargo test` passes including the embedded-browser unit tests.

## E2E Tests

N/A — not applicable for this change. Verification is the scripted desktop
screenshot pass under Integration/Functional Tests plus the manual checklist.

## Manual / cURL Tests

- `curl -A "<shipped UA>" https://www.google.com/` returns the JS homepage
  (no `gbv=1` redirect target content), proving the UA is recognized.
- In the dev app: open Browser pane → google.com → verify layout pixels sit
  only inside the pane; drag the window edge while a page is open → the
  webview tracks; expand the dock → the webview fills the window; open a
  dialog → the webview hides under it (existing `browserViewportVisible`).
