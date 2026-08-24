# Shiki syntax highlighting (issue #295) contract

## Functional Behavior

- [x] Chat code blocks (`Markdown.tsx`'s `CodeBlock`) and diff views (`DiffView.tsx`'s `PatchView`) render plain, escaped text immediately, then upgrade to colorized syntax highlighting once Shiki's grammar for that language loads — first paint is never blocked on the highlighter.
- [x] Colorized output uses the **same** `--syn-*` CSS custom properties chat code, diffs, and the CodeMirror editor already share (`index.css`'s documented "one palette for every piece of code in the app" invariant). Shiki's own bundled theme colors are never emitted directly — only semantic bucket classes (renamed `.hljs-*` → `.stx-*`, same targets). Confirmed live in a real browser: every `.stx-*` class resolves to its own distinct `--syn-*` value in both light and dark mode (see Manual Verification).
- [x] Every language currently supported via `highlight.js` (`bash, c, cpp, csharp, css, dart, diff, dockerfile, elixir, go, graphql, ini, java, javascript, json, kotlin, lua, makefile, markdown, objectivec→objective-c, perl, php, protobuf, python, ruby, rust, scala, scss, sql, swift, typescript, xml, yaml`) has a confirmed Shiki grammar; none is dropped. `toml`, `json5`, `jsonc` gain their own real grammars instead of aliasing to `ini`/`json`. Confirmed against the published `@shikijs/langs` export map, not assumed.
- [x] A language with no grammar (unknown extension) renders plain escaped text, same as today — never throws, never shows raw unescaped HTML.
- [x] A grammar/theme load failure (network hiccup, corrupt dynamic import) falls back to plain escaped text instead of crashing the block.
- [x] `highlightPatch(patch, path)` keeps its exact existing synchronous signature and is called the same way at its existing call site — it now returns structurally-correct, plain-escaped rows instantly; the new async `colorizePatch(patch, path)` produces the same rows with color.
- [x] Diffs/code above a re-tuned size cap (`MAX_HIGHLIGHT_CHARS`, lowered from 400,000 to 50,000 — see Manual Verification for why) render plain escaped text rather than paying Shiki's scope-classification cost at a size where it becomes slow.
- [x] A multi-line construct (block comment, template literal) split across diff-added lines keeps consistent coloring on every line it spans — the property `splitHighlightedLines` used to hand-roll for hljs's blob output, now satisfied natively by Shiki's per-line token arrays.

## Unit Tests (`src/components/highlight.test.ts`)

- [x] `languageFromPath` — unchanged behavior, same cases as today (extension map, dotfile/extensionless conventions, suffix fallback, unknown → `""`).
- [x] `normalizeLang` — recognizes every id in the new canonical list; rejects unknown ids; verifies the `objectivec`/`toml`/`json5`/`jsonc` remaps.
- [x] `colorizeCode(code, lang)` (new, replaces the untested old sync `highlightCode`) — async: known language produces `.stx-*`-classed spans; unknown language returns `escapeHtml(code)`; a forced grammar-load failure falls back to escaped text.
- [x] `colorizePatch` — every existing `highlightPatch` case ported and passing against the new async function: header/hunk/content classification, old/new line numbering, marker stripping, plain-but-escaped unknown-language bodies, multi-line comment spanning added lines, bare fragments with no hunk header, `---`-inside-a-hunk not misread as a header, empty patch, multi-file patches (headers + independent numbering per file), hunk-header undercount recovery, prose-around-a-fragment, no-newline marker not counted against the hunk, size-cap fallback (at the new threshold), and grammar-failure fallback (via `vi.doMock`/`vi.resetModules()` against `shiki/core`, replacing the old `vi.spyOn(hljs, "highlight")`).
- [x] `highlightPatch` (sync) — structural classification/numbering identical to `colorizePatch`'s, bodies always plain-escaped, never throws, no `await` needed.
- [x] `escapeHtml`, `looksLikeDiff` — unchanged, existing cases still pass untouched.
- [x] Line-count parity: rather than a separate helper test, covered through the public API — `colorizePatch`'s blank-context-line case proves a blank source line still produces its own (empty) row rather than collapsing/misaligning, the same guarantee that made removing `splitHighlightedLines` safe. (No private helper was exported solely to unit-test this; the public-surface test gives the same assurance.)

## Integration Tests

- [x] `Markdown.tsx`'s `CodeBlock`: render with a known language, assert plain text appears synchronously (via a *synchronous* `act()`, which reliably captures the pre-colour frame regardless of system load — see commit `test(highlight): poll for colorization...`), then assert colorized `.stx-*` spans appear after the async effect resolves.
- [x] `DiffView.tsx`'s `PatchView`: same plain-then-colorized assertion, plus confirms row kinds/gutter numbers are present from the very first render (not deferred behind colorization). New file — `PatchView` had no test coverage before this PR.
- [x] Changing `CodeBlock`'s `lang`/`body` (or `PatchView`'s `patch`/`path`) mid-life resets to plain immediately rather than showing the previous render's stale colorized HTML while the new content loads.

## Smoke Tests

- [x] `bun run build` succeeds (`tsc -b && vite build`) — confirmed the Vite dynamic-import-per-language chunking resolves cleanly (every language landed in its own chunk, e.g. `typescript-*.js`, `toml-*.js`, `objective-c-*.js`) with no type errors from the new Shiki API usage.
- [x] `bun run test` succeeds (sidecar + `vitest run` + `cargo test --workspace`) on the final, rebased tree — no Rust/protocol changes in this PR, so the cargo suite is an unaffected-baseline confirmation, not a target of the change.
- [x] `bunx vitest run src/components/highlight.test.ts src/components/Markdown.test.tsx src/components/DiffView.test.tsx` green in isolation, and the full 70-file suite green 3 consecutive runs (needed to shake out and fix a real timing flake — see below).

## E2E / Manual Verification

- [~] Live-chat sweep across every previously-supported language: **not possible as originally scoped** — `bun run dev` runs on mock data with no live backend, so a real assistant turn never produces actual code content to render. Substituted with: (a) a direct browser-console probe against the running dev app confirming every `.stx-*` class resolves to its own distinct `--syn-*` color in **both** dark and light mode (not just one), (b) the automated suite exercising real Shiki colorization end-to-end (not mocked) for typescript specifically, and (c) the `normalizeLang`/language-coverage checks confirming grammar availability for the full list against Shiki's real published export map. The literal "type each language into a chat and eyeball it" step is a gap this environment can't close; flagging rather than claiming it as done.
- [~] Real multi-file diff in the Changes view: same constraint as above (no live backend to generate one). Substituted with the automated `colorizePatch` coverage (multi-file patches, multi-line comments spanning added lines) plus a live-browser structural check of `PatchView`'s rendered DOM (gutter, kinds, tint) via a synthetic patch.
- [x] Confirmed the resized/cross-machine-benchmark variance directly: `includeExplanation: true` measured ~30ms at 5K chars, ~110ms at 20K, ~300ms at 50K, climbing into the multi-second range approaching 100K–400K on this machine, with real run-to-run noise at the high end. 50,000 was picked as a conservative margin below where it stays comfortably sub-second, not as a precisely-derived number — worth re-benchmarking on real hardware/content if it ever feels too aggressive or too slow in practice.
- [x] `colorizeCode`'s size-cap fallback confirmed via an automated test with synthetic oversized input, returning instantly rather than paying the classification cost.
- [x] Confirmed `highlight.js` has zero remaining references anywhere in the repo (`grep -rn "highlight.js" src/ package.json` → no matches).
- [x] Rebased onto fresh `origin/main` (7 commits ahead when this branch started); rebase was clean, no conflicts; full suite re-verified green afterward.

### Found during verification, outside the original contract

- **A real test flake, not a production bug**: a fixed `setTimeout(0)` wait in the new integration tests wasn't a reliable way to wait for real dynamic-import + real Shiki tokenization once the *entire* 70-file suite ran concurrently (contending for CPU across many Vite-transform workers) — surfaced only under the full `bun run test`, not the isolated file. Replaced with a poll-until-condition helper. Production code was never affected: `CodeBlock`/`PatchView` have no timeout of their own, they just wait for the real promise, however long that takes.
- **Bundle-size note, not a blocker**: per-language chunk sizes vary far more than expected — most languages land under ~30KB, but a few (`cpp` ~797KB, `graphql` ~371KB, `php`/`objective-c`/`csharp`/`swift` 90–115KB) are heavy, likely because those TextMate grammars embed other languages (e.g. PHP commonly embeds HTML/JS/CSS). Lazy-loading means this cost is paid only when that specific language is actually rendered, but a ~800KB one-time fetch for a C++ snippet is worth knowing about. No before/after byte comparison against the old `highlight.js` baseline was done (would need a separate clean build of `main`); flagging as a good follow-up rather than blocking on it here.
