# Bridge landing site

Marketing site for Bridge: Next.js (App Router), TypeScript, and Tailwind CSS v4. Deploys to Vercel.

## Run it

```bash
bun install
bun run dev     # http://localhost:3000
bun run build   # production build, prerenders /
bun run lint
```

## Layout

- `app/page.tsx` — hero, feature tabs, how-it-works, feature grid, footer
- `app/components/AppMockup.tsx` — the desktop app mockup, rendered from `app/content/scenes.ts`
- `app/components/FeatureTabs.tsx` — the tab strip that switches mockup scenes
- `app/content/site.ts` — external links and the advertised release

## Styling

Tailwind v4 through `@tailwindcss/postcss` (Next.js has no Vite pipeline). Tokens in `app/globals.css` mirror the Graphite palette in `../src/index.css`; keep them in sync. See `AGENTS.md`.
