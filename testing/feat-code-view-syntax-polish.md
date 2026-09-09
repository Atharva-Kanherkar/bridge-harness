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
   plain `--code-foreground`; `.stx-param` resolves to `--color-muted-foreground`.
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

- C1. Seven new tokens exist in both the light (`:root`) and dark
  (`[data-theme="dark"]`-equivalent) blocks: `--syn-variable`, `--syn-property`,
  `--syn-operator`, `--syn-param`, `--syn-regex`, `--syn-addition`,
  `--syn-deletion`. No `--syn-*` token is defined in one theme block and not
  the other.
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
  `stx-param` for the parameter.
- C9. A generic `variable` scope emits `stx-variable`, and it does **not**
  shadow the more specific variable scopes: `variable.parameter` still yields
  `stx-param`, `variable.other.enummember` still yields `stx-number`, and
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
  src/components/CodePanel.test.tsx src/components/fileGlyph.test.ts \
  src/components/palette.test.ts src/components/syntaxCoverage.test.ts
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

## Review round: added contract

Six findings from review, all reproduced against `shiki@4.4.3` + `github-dark`
(the theme `SCOPE_THEME` selects) before changing anything. Four were bugs in
this branch, one was a false claim in a comment, one was missing coverage.

- C27. **Colour is applied per explanation entry, not per token.** Shiki merges
  adjacent same-*styled* runs, and since this file colours by scope rather than
  by Shiki's theme, one token routinely spans scopes we want painted
  differently: `" items."` is one token (whitespace + identifier + accessor),
  `" alpha; }"` is one token, `"**bold**"` is one token. Colouring per token
  from the innermost-last scope painted `items` and `alpha` punctuation-grey
  and made `markup.bold`, `markup.italic` and `markup.strikethrough`
  *permanently unreachable*. Guarded by a round-trip test: stripping the tags
  back out must reproduce the input byte for byte, because this is the one
  change here that could corrupt someone's source.
- C28. **A regex literal is one colour.** Its container `string.regexp` wins
  over the innermost scope. Otherwise `/a+b/g` arrives in four: delimiters
  `punctuation.definition.string.*` (string-green), quantifier
  `keyword.operator.quantifier.regexp` (operator-rose), flags `keyword.other`
  (keyword-violet). The editor's Lezer grammar tags the whole literal `regexp`,
  so this is also what keeps the two renderers agreeing.
- C29. **`=>` is an operator in both renderers.** TypeScript scopes it
  `storage.type.function.arrow`, which hit `storage.type` and came out
  keyword-violet; Lezer tags it `function(punctuation)`, which resolved through
  `punctuation` and came out grey. Neither matched what this branch claimed.
- C30. **No `meta.function-call` rule.** It is a *range* scope spanning the
  whole call, so it labelled the receiver: `obj` in `obj.trim()` came out
  function-blue. `entity.name.function` already covers the callee.
- C31. **Contrast is asserted, not claimed.** A comment claimed 4.5:1 for the
  whole ramp; light `--syn-comment` was 3.74 and `--syn-punct` 4.24, dark
  `--syn-comment` 3.90. `palette.test.ts` now computes WCAG 2.1 contrast for
  every token against every `--code` background in the file — there are
  **three**, not two, because the native vibrancy skin overrides `--code` to
  `#0e0e0d` without overriding the ramp. A separate assertion fails if that set
  of backgrounds ever changes.
- C32. **Every bucket is proven reachable.** `syntaxCoverage.test.ts` runs both
  renderers over fixtures and asserts each `SYNTAX_CLASSES` entry is emitted by
  at least one of them. This is the assertion that would have caught the
  original 23 dead `.tok-*` rules, and it earned its place immediately: it
  found `stx-strike` shipping as a *new* dead bucket. Exemption lists are
  banned by the test's own comment — reach a bucket or delete it.
- C33. Structural claims have tests now. C13/C15/C16 in `CodePanel.test.tsx`
  (guide count per depth, folder glyph follows expanded state, files tint from
  the ramp and directories do not, `aria-expanded` on directories only) and
  C18/C19/C20 in `DiffView.test.tsx` (run edge per kind, banded gutter, no
  doubled edge).
- C34. The hunk band is **one pre-composed opaque class** (`.u-diff-band`), not
  `bg-code` plus a `bg-info/10` alpha utility. tailwind-merge keeps only the
  last `bg-*`, so the layered version silently produced the translucent sticky
  gutter it was meant to fix. `DiffView.test.tsx` asserts the composed class
  and the *absence* of a `bg-*` utility, since that was the actual failure
  mode. Found by writing the C19 test.
- C35. `fileGlyph.ts` / `fileGlyph.test.ts` — neither contains JSX, so neither
  is a `.tsx`.

## Review round: second pass

- C36. **No `meta.decorator` rule.** Like `meta.function-call` in C30 it is a
  *range* scope: it spans the whole decorator, so `@Injectable({ scope: 'x' })`
  had its parens, braces and interior whitespace painted function-blue.
  `entity.name.function` already reaches the callee and
  `punctuation.decorator` the `@`.
- C37. **Inline code is one run.** In markdown the backticks carry
  `punctuation.definition.raw` and came out punctuation grey while the run
  between them carries only `markup.inline.raw` and matched no rule at all, so
  it rendered as bare text. Both are bucketed `stx-string` now, with the
  delimiter rule ahead of the generic `punctuation` one — the same ordering
  comments and strings already rely on. The editor reaches the same bucket
  through `t.monospace`.
- C38. **`PatchView` owns its background.** The pinned gutter, the fold bar and
  `u-diff-band` are all pre-composed against `--color-code`, and a caller
  rendering the patch on `bg-card` (`#ffffff`/`#0f0f0f` against `--code`'s
  `#f3f3f1`/`#0a0a0a`) painted them as mismatched rectangles in the transcript.
  The root carries `bg-code` so omission cannot get it wrong.
- C39. **Contrast is asserted per class, not per token.** C31's assertion read
  `--syn-*` declarations, which silently skipped anything not written as
  six-digit lowercase hex and never covered the buckets coloured from
  `--color-muted-foreground`, `--color-destructive` or `--color-success`, or
  the three that inherit `--code-foreground`. It now resolves each `.stx-*`
  rule's own `color` through `@theme` to a hex, composites the rule's own tint
  over each `--code` background, and *fails* on anything it cannot resolve.
  That immediately found two real ones: paper `.stx-addition` was 3.91 and
  `.stx-deletion` 4.13 against their own tinted surface, which is why those two
  buckets now take `--syn-addition`/`--syn-deletion` — a step darker than the
  chrome status colours — instead of `--color-success`/`--color-destructive`.
- C40. **`stx-param` matches `--syn-param`.** The class was plural and the token
  singular; nothing broke, but the pair is the one naming convention the three
  renderers share.
