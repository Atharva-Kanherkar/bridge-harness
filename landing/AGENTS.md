<!-- BEGIN:nextjs-agent-rules -->

# This is NOT the Next.js you know

This version has breaking changes — APIs, conventions, and file structure may all differ from your training data. Read the relevant guide in `node_modules/next/dist/docs/` (resolved from this file's directory; in monorepos the `next` package may not be visible from the repo root) before writing any code. Heed deprecation notices.

This block is written and re-added by `next dev` — verify at `node_modules/next/dist/server/lib/generate-agent-files.js`. Removing it from a diff only re-creates the uncommitted change; committing it with your work keeps the tree clean.

<!-- END:nextjs-agent-rules -->

# Landing site conventions

- Tailwind CSS v4 only, configured CSS-first in `app/globals.css` (`@theme`, `@layer base`). No `tailwind.config.js`, no CSS Modules, no CSS-in-JS, no inline `style` for anything a utility can express.
- Next.js has no Vite pipeline, so `@tailwindcss/postcss` in `postcss.config.mjs` is the only supported Tailwind v4 integration here. Keep that file to the single Tailwind plugin.
- The tokens in `app/globals.css` mirror the Graphite (dark) palette in `../src/index.css` value for value, using the same variable names. When the app palette changes, update this file in the same change so the mockup stays accurate.
- The desktop mockup in `app/components/AppMockup.tsx` is data-driven: every tab state lives in `app/content/scenes.ts`. Add or edit scenes there rather than branching inside the component.
- External links and the advertised release live in `app/content/site.ts`.
