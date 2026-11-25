---
name: bridge-ui
description: Build or change any Bridge screen, pane, dialog, row or control so it matches the rest of the app. Use before writing or editing JSX with className in src/, when adding a feature that has UI, when a lint error names shadcn/* or bridge/* rules, or when asked to make something look consistent, minimal or polished.
---

# Building Bridge UI

Bridge looks like one app because feature code does not draw controls. Read
`docs/design/ui-rules.md` once; this is the working order.

## Before you write JSX

1. Find the nearest existing screen that does the same job and open it. Match
   its structure: the same heading, the same row, the same empty state.
2. List the controls you need and find each in `src/components/ui/` (the table
   in `docs/design/ui-rules.md` maps needs to primitives). Read the primitive's
   variants and sizes before choosing.
3. If one is missing, add the variant or the primitive to `src/components/ui/`
   first and commit it on its own. Pull from the registry when it exists there:
   `bunx shadcn@latest add @coss/<name>`, then restyle it with Bridge tokens.

## While you write

- Primitives get layout classes only: width, flex, grid, margin, position,
  `truncate`. Never padding, colour, type size, radius or shadow.
- No raw `<button>`, `<input>`, `<select>`, `<textarea>`. No `title=` for a
  hint: `<Tip label>` or `tooltip=` on `Button`.
- Type sizes are the named scale only: `text-micro`, `text-caption`,
  `text-meta`, `text-ui`, `text-body`, `text-message`, `text-heading`,
  `text-title`. Colours are theme tokens only.
- Spacing between things comes from the parent (`gap-*`, `space-y-*`), on the
  Tailwind scale.
- One accent of emphasis per view. Secondary actions are `quiet` or `ghost`;
  at most one `default` (filled) button is visible at a time.
- Every pane handles four states: loading (`Skeleton*`), empty (`PaneState`
  or `Empty`), error (`Alert` or `PaneState role="alert"`), and content.
- An icon-only control has an `aria-label` and a tooltip.

## Before you say it is done

```bash
bun run lint                 # must pass; each error names the fix
bunx vitest run src/designSystem.test.ts
bun run build
```

Then look at it. Run `bun run dev`, open the screen in dark and in light, and
check hover, focus (Tab through it), disabled and the narrowest width the
pane allows. Attach before and after screenshots to the PR.

If lint reports fewer violations than the baseline, run `bun run lint:update`
and commit `lint-baseline.json`. Never raise a count in it.
