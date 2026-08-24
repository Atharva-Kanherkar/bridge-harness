# Bridge Docs

A local, self-contained documentation site for Bridge. One HTML file, no build
step, no framework — styled with Bridge's own "Graphite & Paper" design tokens
so it looks and feels like the app it documents.

## Run it

```bash
bun run docs
```

Opens on `http://localhost:3000` (or the next free port `serve` picks). You
can also just open `docs-site/index.html` directly in a browser — everything
except the Google Fonts request works fully offline.

## What's here

Three pages are fully written: **Introduction**, **Session forest**, and
**`bridge exec --json`**. Every other item in the sidebar is a real nav entry
that resolves to an honest "not drafted yet" placeholder rather than a dead
link — the structure is intentionally locked before the remaining content is.

The Session forest page's diagram is hand-authored inline SVG following the
same visual grammar as Bridge's in-app diagram renderer
(`src/components/DiagramFigure.tsx`) and the locked skill at
`.claude/skills/bridge-diagram/`. When adding a diagram to a new page, follow
that skill rather than inventing a new visual style.

## Why plain HTML instead of the Vite/React app

This site is deliberately not part of the `src/` frontend: it has no runtime
dependency on Tauri, and "opens on the web" is meant literally — anyone can
point a static file server at this directory. The tokens in the `<style>`
block are copied by hand from `src/index.css` (the "Light · paper" / "Dark ·
graphite" blocks) rather than imported, since there's no build step to share
them through. If they drift, diff against `src/index.css` and resync.
