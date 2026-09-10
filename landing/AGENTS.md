<!-- BEGIN:nextjs-agent-rules -->

# This is NOT the Next.js you know

This version has breaking changes — APIs, conventions, and file structure may all differ from your training data. Read the relevant guide in `node_modules/next/dist/docs/` (resolved from this file's directory; in monorepos the `next` package may not be visible from the repo root) before writing any code. Heed deprecation notices.

This block is written and re-added by `next dev` — verify at `node_modules/next/dist/server/lib/generate-agent-files.js`. Removing it from a diff only re-creates the uncommitted change; committing it with your work keeps the tree clean.

<!-- END:nextjs-agent-rules -->

# Landing site conventions

- Tailwind CSS v4 only, configured CSS-first in `app/globals.css` (`@theme`, `@layer base`). No `tailwind.config.js`, no CSS Modules, no CSS-in-JS, no inline `style` for anything a utility can express.
- Next.js has no Vite pipeline, so `@tailwindcss/postcss` in `postcss.config.mjs` is the only supported Tailwind v4 integration here. Keep that file to the single Tailwind plugin.
- The tokens in `app/globals.css` mirror the Graphite (dark) palette in `../src/index.css` value for value, using the same variable names. When the app palette changes, update this file in the same change so the mockup stays accurate.
- The hero shows real captures of the app, not a mockup. `app/content/hero.ts` lists one capability per capture, and the files live in `public/screens/*.webp`. They are taken from the app's own preview mode (`bun run dev` in the repo root, accept the model setup wizard, add the `dark` class to `html`, open the mock orchestrator session) at 1600×1000 with a 2× device scale factor, then encoded with `cwebp -q 88`. Recapture in the same change as any app redesign, or the hero becomes a lie.
- `app/components/HeroFrame.tsx` is the switcher: five tabs, one active capture, a progress bar that fills over seven seconds before advancing. Hovering or focusing the switcher holds the current capability, `prefers-reduced-motion` disables auto-advance, and the first capture is server-rendered so the page is complete without JavaScript.
- The loop section's mini mocks still render from `app/components/MockEntry.tsx` with entries typed in `app/content/scenes.ts`; keep those aligned with the app's entry styles.
- The docs section renders the repository's own markdown. `app/content/docs.ts` is a curated manifest, and `app/lib/renderDoc.ts` reads each file from `../docs` at build time and rewrites its relative links: a target in the manifest becomes an internal route, anything else becomes a GitHub blob link that opens in a new tab. Adding a doc means adding a manifest entry, not copying prose.
- Because the docs are read from `../docs`, the Vercel project must build with the whole repository available, not the `landing/` directory alone. If a deploy ever ships an empty docs section, that is the cause.
- `.doc-prose` in `app/globals.css` is a deliberate exception to utilities-first styling, for the same reason the app keeps a `.md` class: rendered markdown needs descendant selectors.
- Blog posts live in `app/content/blog.ts` as typed blocks. If someone wants to write long-form in markdown, MDX is the upgrade path; it was not worth the toolchain for prose this short.
- External links and the advertised release live in `app/content/site.ts`.
