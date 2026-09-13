# AGENTS.md

Guidance for AI agents and contributors working in this repository.

Bridge is a desktop app: **Tauri 2** (Rust) shell hosting a **React 18 + TypeScript + Vite** frontend. UI lives in `src/`; the native side lives in `src-tauri/`.

## Styling: Tailwind CSS v4 only — non-negotiable

**This project styles exclusively with Tailwind CSS v4. There are no exceptions.**

- **Tailwind v4, not v3.** We use the v4 engine via `@tailwindcss/vite` and the CSS-first config in `src/index.css` (`@import "tailwindcss"`, `@theme`, `@custom-variant`, `@layer`, `@utility`, `--alpha()`). Do **not** introduce a `tailwind.config.js`, PostCSS plugin chain, or any v3-era pattern. Configure through the `@theme` block in `src/index.css`.
- **Utilities first.** Style components with Tailwind utility classes directly in the JSX (`className="..."`). This is the default and strongly preferred way to style anything.
- **One stylesheet.** `src/index.css` is the single global stylesheet and the only place for design tokens, `@layer base` resets, keyframes, and the handful of custom classes that utilities genuinely cannot express (e.g. the markdown renderer's descendant selectors, complex keyframe animations). Do **not** add new standalone `.css` files and import them ad hoc — fold anything shared into `src/index.css`.
- **Tokens over hardcoded values.** Reach for theme tokens (`bg-background`, `text-foreground`, `border-border`, `font-display`, …) before arbitrary values. Use arbitrary values (`h-[2px]`, `bg-[#020204]`) only when no token fits.
- **Banned:** other CSS frameworks (Bootstrap, MUI, Chakra, …), CSS-in-JS runtimes (styled-components, Emotion, stitches), CSS Modules, Sass/Less, and inline `style={{…}}` for anything a utility or token can do. Inline `style` is acceptable only for values that must be computed at runtime (e.g. per-element positions in `SpaceBackground`).

If a design need seems to require stepping outside Tailwind, that is a signal to add a token or a `@utility`/`@layer` rule in `src/index.css` — not to reach for another tool.

## Development

```bash
bun install          # install deps
bun run dev          # vite dev server on http://127.0.0.1:1420
bun run tauri dev    # full desktop app (Rust + webview)
bun run build        # tsc -b && vite build
bun run check        # tsc -b + cargo check
bun run test         # vitest run + cargo test
```

**Claude models require Node.** Claude runs through the Claude Agent SDK via a
Node sidecar (`sidecar/claude-agent/`), not `claude -p`. The root `bun install`
installs its workspace dependencies; keep `node` (18+) on PATH.
The Rust adapter finds the sidecar via `BRIDGE_CLAUDE_SIDECAR`, then a copy next
to the executable, then the in-repo path.

Always run `bun run build` and `bun run test` before opening or merging a PR; both must be green.

## Conventions

- **Layout:** `App.tsx` owns app state and composes `BridgeSidebar`, the `ComposerPill`, `AgentConversation`, and dialogs. Prefer small, prop-driven presentational components in `src/components/`.
- **Icons:** `lucide-react`. Keep unused icon imports out (the build is strict).
- **Fonts:** Geist Variable (`font-sans`) for body, Bricolage Grotesque Variable (`font-display`) for headings, Geist Mono Variable (`font-mono`) for code — loaded via `@fontsource-variable/*` in `main.tsx` and wired through `@theme` tokens.
- **Glass:** macOS-style frosted glass comes from the `.u-glass` (panels), `.u-glass-popover` (menus/toasts/dialogs), `.u-glass-soft` (cards/rows/inputs), and `.u-segmented` / `.u-segmented-item` (segmented controls) classes in `src/index.css`. Prefer them over hand-rolled `bg-white/[0.0x] backdrop-blur-*` combinations.
- **Transcript:** the conversation surface behaves the same for every agent, and [`docs/transcript-behavior-contract.md`](docs/transcript-behavior-contract.md) says exactly how: one thinking component, grouping invariants, stream-state meanings, and a harness id that is display and never behavior.
- **Observability:** what Bridge records, what it deliberately does not, and how to read or export it is in [`docs/observability.md`](docs/observability.md). The transcript pane's facet, turn and problem rules live there too.
- **Tests:** colocated `*.test.ts(x)` run under Vitest. Add coverage for logic in `utils`, `conversation`, `observability`, and `usage`.
- **Commits:** Conventional Commits (`feat:`, `fix:`, `refactor:`, `chore:`). Describe the change; don't cite issue/PR numbers in code or messages.

## Memory surface

Memory has exactly **one** surface: the Memory screen (`src/components/MemoryDialog.tsx`), a canvas view beside the sidebar like Projects, rendered in the app's normal Graphite & Paper chrome with the same tokens as every other screen. Do not build a second memory UI, a separate dark-only memory surface, or a parallel token family for it — that was tried (the "Memory Core" constellation) and removed. Analytics (recall stats, packet budget, consolidation log) live inside its Activity tab, achromatic like the rest of the chrome.
