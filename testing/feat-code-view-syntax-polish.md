# feat/code-view-syntax-polish — test contract

Locked before implementation.

## What is wrong today

Every piece of code in Bridge shares one palette (`--syn-*` in `src/index.css`),
consumed by three renderers: Shiki's scope classifier (`.stx-*`, chat blocks and
diffs), CodeMirror's `classHighlighter` (`.tok-*`, the Code tab), and the diff
viewer's row tints. The plumbing is sound; the palette and the surrounding
chrome are what read as flat:

1. **Most of a file is uncoloured.** The palette has eight hues and none of them
   is for an identifier. `.tok-variableName`, `.tok-definition`,
   `.tok-propertyName`, `.tok-attributeName` and `.tok-labelName` all resolve to
   plain `--code-foreground`; `.stx-params` resolves to `--color-muted-foreground`.
   In `highlight.ts`'s `SCOPE_RULES` there is no rule at all for `variable`,
   `entity.other.attribute-name`, `variable.other.property` or
   `constant.regexp`, so a Shiki token carrying only those scopes falls through
   `classifyScope` and renders as bare text.
1b. **Half the editor's stylesheet is dead.** Found while implementing, and the
   bigger half of the problem. `@lezer/highlight`'s `classHighlighter` maps only
   29 class names, and a tag with no spec of its own resolves through its `set`
   to a base tag that does have one. Measured against the installed version:
   `tok-function`, `tok-tagName`, `tok-attributeName`, `tok-regexp`,
   `tok-escape`, `tok-special`, `tok-character`, `tok-null`, `tok-unit`,
   `tok-standard`, `tok-separator`, `tok-derefOperator`, `tok-squareBracket`,
   `tok-brace`, `tok-paren`, `tok-modifier`, `tok-controlKeyword`,
   `tok-operatorKeyword`, `tok-definitionKeyword`, `tok-moduleKeyword`,
   `tok-processingInstruction`, `tok-documentMeta` and `tok-strikethrough` can
   **never be emitted**, so every rule `index.css` wrote for them was inert. The
   user-visible consequences: no function call or declaration is distinguishable
   from a plain identifier, JSX/HTML element names wear the *type* colour, and
   `tok-inserted`/`tok-deleted` are emitted but unstyled. Retuning the palette
   alone would have repainted rules that never fire.
2. **Operators are punctuation.** `keyword.operator` → `stx-punct` and
   `.tok-operator` → `--syn-punct`, so `=>`, `??`, `===` are the same grey as `;`
   and `,`.
3. **The file tree has no structure.** Directory and file rows are the same
   monospace muted text; there is no folder glyph, no per-file-type glyph, and no
   indent guide, so depth is carried only by left padding.
4. **The editor gutter is not a column.** `.cm-gutters` uses the same background
   as `.cm-content`, separated by one hairline.
5. **Diff rows tint at 8%** with no run boundary, so an add/del block has no
   edge against surrounding context.

## Scope

Palette and presentation only. No change to language detection, the grammar
loader, `parsePatch`, buffer/save semantics, or any protocol surface.

## Contract

### Palette — `src/index.css`

- C1. Five new tokens exist in both the light (`:root`) and dark
  (`[data-theme="dark"]`-equivalent) blocks: `--syn-variable`, `--syn-property`,
  `--syn-operator`, `--syn-param`, `--syn-regex`. No `--syn-*` token is defined
  in one theme block and not the other.
- C2. Every `--syn-*` token retains its existing hue family. Chroma may rise;
  hue may not be reassigned (keyword stays violet, string green, number amber,
  function blue, type magenta, tag cyan). This is a refinement of Graphite &
  Paper, not a new palette.
- C3. Chrome stays achromatic. No `--color-*` chrome token gains saturation, and
  no new accent colour is introduced outside the `--syn-*` family.

### Scope classifier — `src/components/highlight.ts`

- C4. `colorizeCode("obj.prop = 1;", "typescript")` emits `stx-property`.
  Asserted on member *access*, not on object-literal keys: TextMate's
  TypeScript grammar emits `{ alpha: ` as a single `meta.object.member` run
  with no scope of its own on the key, so no classifier rule can reach it.
- C5. `colorizeCode("const f = (a, b) => a + b;", "typescript")` emits
  `stx-operator` for `=>` / `+`, and that class is distinct from `stx-punct`.
- C6. `colorizeCode('const re = /a+b/g;', "typescript")` emits `stx-regex`.
- C7. `colorizeCode('<div className="x" />', "tsx")` emits `stx-property` for
  the attribute name.
- C8. `colorizeCode("function f(alpha) { return alpha; }", "typescript")` emits
  `stx-params` for the parameter.
- C9. A generic `variable` scope emits `stx-variable`, and it does **not**
  shadow the more specific variable scopes: `variable.parameter` still yields
  `stx-params`, `variable.other.enummember` still yields `stx-number`, and
  `variable.language` (`this`, `self`) still yields `stx-keyword`. See C26 for
  why `variable.other.constant` is deliberately *not* on that list.
- C10. Every class name `classifyScope` can return has a matching rule in
  `src/index.css`. Asserted mechanically by reading the stylesheet, so a new
  bucket cannot ship without its colour.
- C11. Unchanged: an unknown language, an input past `MAX_HIGHLIGHT_CHARS`, and
  `highlightPatch` before `colorizePatch` resolves all still emit zero `stx-`
  classes.

### File tree — `src/components/fileGlyph.tsx` (new) + `CodePanel.tsx`

- C12. `glyphFor(path)` is a pure mapping from a path to `{ Icon, tint }`, keyed
  off `languageFromPath` plus a small filename table, so the tree and the
  highlighter never disagree about what a file is.
- C13. A directory row renders a folder glyph that reflects its expanded state;
  a file row renders its type glyph. Both are `aria-hidden`.
- C14. `tint` is always a `--syn-*`-derived class, never a chrome token and
  never a raw hex value.
- C15. A row at depth _n_ renders exactly _n_ indent guides, and the row's left
  offset comes from those guides rather than a computed inline `paddingLeft`.
- C16. A directory name is visually distinguishable from a file name at the same
  depth (weight/foreground, not colour).
- C17. Unchanged: clicking a directory toggles, clicking a file opens,
  `MAX_TREE_ROWS` still caps painted rows, and `aria-expanded` is present on
  directories only.

### Diff rows — `src/components/DiffView.tsx`

- C18. An `add` row and a `del` row each carry a left run edge in their own
  colour, in addition to the body tint.
- C19. A `hunk` row reads as a band across the full row width, not a tinted
  body beside an untinted gutter.
- C20. The pinned gutter is separated from the body by its own edge, so line
  numbers read as a column under horizontal scroll.
- C21. Unchanged: `splitHunks` grouping, fold-bar counts, quote-hunk affordance
  and its `aria-label`, and gutter line numbers (`highlightPatch` numbering
  tests must pass untouched).

### Editor chrome — `src/index.css`

- C22. `.cm-gutters` has a background distinct from `--color-code`.
- C23. `.cm-activeLineGutter` marks the active line with an edge, not only a
  fill.
- C24. No new dependency is added. `baseExtensions` changes in exactly one
  way: `syntaxHighlighting(classHighlighter)` becomes
  `syntaxHighlighting(bridgeHighlighter)`, a `tagHighlighter` defined in
  `editor/highlighter.ts` that covers the full tag vocabulary and emits the
  shared `stx-*` buckets. (Amended from the locked version, which assumed the
  editor's class plumbing was sound — finding 1b above is why.)
- C25. `index.css` contains no `.tok-*` rule afterwards. Any that survived
  would be dead by construction, since nothing emits that family any more.
- C26. Neither renderer buckets `variable.other.constant` /
  `constant(variableName)` as a literal. TextMate gives that scope to **every**
  `const` binding in TypeScript, so treating "constant" as a value paints most
  of a TS file in the number hue. Regression-guarded, because the first
  implementation did exactly that.

## Verification

```bash
bunx vitest run src/components/highlight.test.ts src/components/DiffView.test.tsx \
  src/components/CodePanel.test.tsx src/components/fileGlyph.test.tsx src/components/palette.test.ts
bun run build
bun run test
```

C1/C2/C3/C10/C14/C25 are asserted by `src/components/palette.test.ts`, which
parses `src/index.css` — the design rules are enforced by a test rather than by
review. That is the direct answer to finding 1b: a class/colour mismatch went
unnoticed for as long as it did because nothing but review was checking, and
review is exactly what missed it.

`src/components/macosNavigation.test.tsx` fails on `origin/main` (2 cases,
appearance-radio keyboard nav) and still fails here. Verified at the base commit
in a scratch worktree; out of scope for this branch.
