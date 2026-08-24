# Shiki syntax highlighting (issue #295) contract

## Functional Behavior

- [ ] Chat code blocks (`Markdown.tsx`'s `CodeBlock`) and diff views (`DiffView.tsx`'s `PatchView`) render plain, escaped text immediately, then upgrade to colorized syntax highlighting once Shiki's grammar for that language loads — first paint is never blocked on the highlighter.
- [ ] Colorized output uses the **same** `--syn-*` CSS custom properties chat code, diffs, and the CodeMirror editor already share (`index.css`'s documented "one palette for every piece of code in the app" invariant). Shiki's own bundled theme colors are never emitted directly — only semantic bucket classes (renamed `.hljs-*` → `.stx-*`, same targets).
- [ ] Every language currently supported via `highlight.js` (`bash, c, cpp, csharp, css, dart, diff, dockerfile, elixir, go, graphql, ini, java, javascript, json, kotlin, lua, makefile, markdown, objectivec→objective-c, perl, php, protobuf, python, ruby, rust, scala, scss, sql, swift, typescript, xml, yaml`) has a confirmed Shiki grammar; none is dropped. `toml`, `json5`, `jsonc` gain their own real grammars instead of aliasing to `ini`/`json`.
- [ ] A language with no grammar (unknown extension) renders plain escaped text, same as today — never throws, never shows raw unescaped HTML.
- [ ] A grammar/theme load failure (network hiccup, corrupt dynamic import) falls back to plain escaped text instead of crashing the block.
- [ ] `highlightPatch(patch, path)` keeps its exact existing synchronous signature and is called the same way at its existing call site — it now returns structurally-correct, plain-escaped rows instantly; the new async `colorizePatch(patch, path)` produces the same rows with color.
- [ ] Diffs/code above a re-tuned size cap (`MAX_HIGHLIGHT_CHARS`, lowered from 400,000 to 50,000 — see Manual Verification for why) render plain escaped text rather than paying Shiki's scope-classification cost at a size where it becomes slow.
- [ ] A multi-line construct (block comment, template literal) split across diff-added lines keeps consistent coloring on every line it spans — the property `splitHighlightedLines` used to hand-roll for hljs's blob output, now satisfied natively by Shiki's per-line token arrays.

## Unit Tests (`src/components/highlight.test.ts`)

- [ ] `languageFromPath` — unchanged behavior, same cases as today (extension map, dotfile/extensionless conventions, suffix fallback, unknown → `""`).
- [ ] `normalizeLang` — recognizes every id in the new canonical list; rejects unknown ids; verifies the `objectivec`/`toml`/`json5`/`jsonc` remaps.
- [ ] `colorizeCode(code, lang)` (new, replaces the untested old sync `highlightCode`) — async: known language produces `.stx-*`-classed spans; unknown language returns `escapeHtml(code)`; a forced grammar-load failure falls back to escaped text.
- [ ] `colorizePatch` — every existing `highlightPatch` case ported and passing against the new async function: header/hunk/content classification, old/new line numbering, marker stripping, plain-but-escaped unknown-language bodies, multi-line comment spanning added lines, bare fragments with no hunk header, `---`-inside-a-hunk not misread as a header, empty patch, multi-file patches (headers + independent numbering per file), hunk-header undercount recovery, prose-around-a-fragment, no-newline marker not counted against the hunk, size-cap fallback (at the new threshold), and grammar-failure fallback (replacing the old `vi.spyOn(hljs, "highlight")` mocks with an equivalent forced-failure mock of the Shiki call path).
- [ ] `highlightPatch` (sync) — structural classification/numbering identical to `colorizePatch`'s, bodies always plain-escaped, never throws, no `await` needed.
- [ ] `escapeHtml`, `looksLikeDiff` — unchanged, existing cases still pass untouched.
- [ ] Line-count parity: a token-line helper test proving blank lines and a missing trailing newline still produce exactly one output row per input line (the property that made removing `splitHighlightedLines` safe).

## Integration Tests

- [ ] `Markdown.tsx`'s `CodeBlock`: render with a known language, assert plain text appears synchronously, then assert colorized `.stx-*` spans appear after the async effect resolves (jsdom + `act`/microtask flush).
- [ ] `DiffView.tsx`'s `PatchView`: same plain-then-colorized assertion, plus confirms row kinds/gutter numbers are present from the very first render (not deferred behind colorization).
- [ ] Changing `CodeBlock`'s `lang`/`body` (or `PatchView`'s `patch`/`path`) mid-life resets to plain immediately rather than showing the previous render's stale colorized HTML while the new content loads.

## Smoke Tests

- [ ] `bun run build` succeeds (`tsc -b && vite build`) — confirms the Vite dynamic-import-per-language chunking resolves cleanly and there are no type errors from the new Shiki API usage.
- [ ] `bun run test` succeeds (sidecar + `vitest run` + `cargo test --workspace`) — no Rust/protocol changes in this PR, so the cargo suite is an unaffected-baseline check.
- [ ] `bunx vitest run src/components/highlight.test.ts src/components/Markdown.test.tsx src/components/DiffView.test.tsx` (or whichever component test files cover these) green in isolation.

## E2E / Manual Verification

- [ ] `bun run dev`, open a chat with fenced code blocks in several of the previously-supported languages (at least: typescript/tsx, python, rust, bash, yaml, json, a Dockerfile snippet, and one language new to a real grammar — toml) — colors match the app's existing palette (compare against the CodeMirror editor's coloring of the same snippet, since both must draw from the same `--syn-*` tokens) in both light and dark mode.
- [ ] Open the Changes/diff view on a real multi-file diff — verify old/new line numbers, add/del tint, and per-line coloring, including at least one hunk with a multi-line comment or string spanning an added line.
- [ ] Paste or generate a diff/code block near and above the new 50,000-character cap — confirm the plain-escaped fallback engages instead of a multi-second stall. (Cap chosen from empirical benchmarking in this session: `codeToTokens` with `includeExplanation: true`, the mode needed to classify tokens into `--syn-*` buckets, measured roughly 100–600ms at 20–50K characters and climbed into the multi-second range approaching 100K–400K on this machine, with real run-to-run variance; 50,000 was picked as a conservative margin below where it stays comfortably sub-second rather than as an exact derived number — worth re-benchmarking on real hardware/content if this ever feels too aggressive or too slow in practice.)
- [ ] Confirm `package.json` no longer depends on `highlight.js` anywhere (`grep -rn "highlight.js" src/` returns nothing).
- [ ] Review the final diff against fresh `origin/main` for unrelated changes.
