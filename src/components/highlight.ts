import { createHighlighterCore, type HighlighterCore, type LanguageInput } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";

/**
 * The public language vocabulary. This exact set of ids is a shared contract
 * with `editor/language.ts`'s `LOADERS` map — that file is keyed by whatever
 * `languageFromPath`/`normalizeLang` produce here, specifically so the
 * CodeMirror editor and the diff/chat viewer never disagree about what
 * language a file is in. Do not rename or split an id here without adding a
 * matching change there; `SHIKI_GRAMMAR_FOR` below is where a *better*
 * grammar gets used without touching this public vocabulary at all.
 */
const CANONICAL_LANGS = new Set([
  "bash", "c", "cpp", "csharp", "css", "dart", "diff", "dockerfile", "elixir", "go",
  "graphql", "ini", "java", "javascript", "json", "kotlin", "lua", "makefile",
  "markdown", "objectivec", "perl", "php", "protobuf", "python", "ruby", "rust",
  "scala", "scss", "sql", "swift", "typescript", "xml", "yaml",
]);

const LANG_ALIASES: Record<string, string> = {
  js: "javascript", jsx: "javascript", mjs: "javascript", cjs: "javascript",
  ts: "typescript", tsx: "typescript", mts: "typescript", cts: "typescript",
  py: "python", rs: "rust", sh: "bash", zsh: "bash", shell: "bash", console: "bash",
  yml: "yaml", html: "xml", vue: "xml", svelte: "xml", md: "markdown",
  "c++": "cpp", objc: "objectivec", patch: "diff", toml: "ini",
  rb: "ruby", kt: "kotlin", kts: "kotlin", cs: "csharp", ex: "elixir", exs: "elixir",
  gql: "graphql", pl: "perl", sass: "scss", m: "objectivec", mm: "objectivec",
  h: "c", hpp: "cpp", cc: "cpp", cxx: "cpp", proto: "protobuf", make: "makefile",
  jsonc: "json", json5: "json", ndjson: "json", mdx: "markdown", plaintext: "", text: "",
};

/**
 * Where Shiki's real grammar is more accurate than the public id above would
 * suggest — translated only at load time, so `normalizeLang`'s output (and
 * therefore the editor's contract) never changes. `javascript`/`typescript`
 * route to Shiki's `jsx`/`tsx` grammars unconditionally: both are strict
 * supersets that tokenize plain, JSX-free code identically to the
 * non-JSX grammar (verified directly), so there's no plain-code downside —
 * only the alternative, misparsing `<Component>` as a comparison
 * expression under the plain `typescript`/`javascript` grammar.
 */
const SHIKI_GRAMMAR_FOR: Partial<Record<string, string>> = {
  objectivec: "objective-c",
  ini: "toml",
  javascript: "jsx",
  typescript: "tsx",
};

/**
 * Every grammar (and the one theme we load) is a dynamic import: rendering a
 * TypeScript block must not also pay for the PHP grammar. Literal per-language
 * import specifiers (not a templated path) are what let Vite split each one
 * into its own chunk — the same shape `editor/language.ts` already uses for
 * CodeMirror's lazily-loaded language support. Keyed by Shiki's own grammar
 * ids, which is a different (and finer) vocabulary than `CANONICAL_LANGS`
 * above — `SHIKI_GRAMMAR_FOR` bridges the two.
 */
const LANG_LOADERS: Record<string, () => LanguageInput> = {
  bash: () => import("shiki/langs/bash.mjs"),
  c: () => import("shiki/langs/c.mjs"),
  cpp: () => import("shiki/langs/cpp.mjs"),
  csharp: () => import("shiki/langs/csharp.mjs"),
  css: () => import("shiki/langs/css.mjs"),
  dart: () => import("shiki/langs/dart.mjs"),
  diff: () => import("shiki/langs/diff.mjs"),
  dockerfile: () => import("shiki/langs/dockerfile.mjs"),
  elixir: () => import("shiki/langs/elixir.mjs"),
  go: () => import("shiki/langs/go.mjs"),
  graphql: () => import("shiki/langs/graphql.mjs"),
  java: () => import("shiki/langs/java.mjs"),
  json: () => import("shiki/langs/json.mjs"),
  jsx: () => import("shiki/langs/jsx.mjs"),
  kotlin: () => import("shiki/langs/kotlin.mjs"),
  lua: () => import("shiki/langs/lua.mjs"),
  makefile: () => import("shiki/langs/makefile.mjs"),
  markdown: () => import("shiki/langs/markdown.mjs"),
  "objective-c": () => import("shiki/langs/objective-c.mjs"),
  perl: () => import("shiki/langs/perl.mjs"),
  php: () => import("shiki/langs/php.mjs"),
  protobuf: () => import("shiki/langs/protobuf.mjs"),
  python: () => import("shiki/langs/python.mjs"),
  ruby: () => import("shiki/langs/ruby.mjs"),
  rust: () => import("shiki/langs/rust.mjs"),
  scala: () => import("shiki/langs/scala.mjs"),
  scss: () => import("shiki/langs/scss.mjs"),
  sql: () => import("shiki/langs/sql.mjs"),
  swift: () => import("shiki/langs/swift.mjs"),
  toml: () => import("shiki/langs/toml.mjs"),
  tsx: () => import("shiki/langs/tsx.mjs"),
  xml: () => import("shiki/langs/xml.mjs"),
  yaml: () => import("shiki/langs/yaml.mjs"),
};

/** Extensionless files that still have an obvious language. */
const FILENAME_LANGS: Record<string, string> = {
  dockerfile: "dockerfile", containerfile: "dockerfile",
  makefile: "makefile", gnumakefile: "makefile", justfile: "makefile",
  gemfile: "ruby", rakefile: "ruby", podfile: "ruby", brewfile: "ruby",
  ".bashrc": "bash", ".zshrc": "bash", ".bash_profile": "bash", ".profile": "bash",
  ".env": "ini", ".editorconfig": "ini",
};

/**
 * Best-guess language for a file path.
 *
 * Whole-filename conventions first (Dockerfile, Makefile, dotfiles), then the
 * extension: the order matters, because a dotfile's only "extension" is its
 * whole name, so an extension-first pass would return "" for `.env` and never
 * reach the map. Returns "" when we have no grammar, which callers read as
 * "render it plain".
 */
export function languageFromPath(path: string): string {
  const name = path.split(/[\\/]/).pop()?.toLowerCase() ?? "";
  if (!name) return "";
  const byName = FILENAME_LANGS[name];
  if (byName) return normalizeLang(byName);
  // ".gitignore" is a dotfile, not an extension; only split on a real one.
  const dot = name.lastIndexOf(".");
  if (dot > 0) {
    const direct = normalizeLang(name.slice(dot + 1));
    if (direct) return direct;
    // "schema.prisma.bak", "config.yaml.tmpl" — try the extension underneath.
    const inner = name.slice(0, dot);
    const innerDot = inner.lastIndexOf(".");
    if (innerDot > 0) return normalizeLang(inner.slice(innerDot + 1));
  }
  return "";
}

/** Normalize to one of `CANONICAL_LANGS`, or `""` when we have no grammar. */
export function normalizeLang(lang: string): string {
  const key = lang.trim().toLowerCase();
  if (!key) return "";
  const aliased = LANG_ALIASES[key] ?? key;
  return CANONICAL_LANGS.has(aliased) ? aliased : "";
}

export function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

const DIFF_LINE = /^(\+\+\+|---|@@|\+[^+]|-[^-])/m;

/** Heuristic: does this tool output look like a unified diff/patch? */
export function looksLikeDiff(text: string): boolean {
  if (!text) return false;
  const sample = text.slice(0, 4000);
  if (/^@@ /m.test(sample)) return true;
  if (/^(diff --git|--- |\+\+\+ )/m.test(sample)) return true;
  const lines = sample.split("\n").filter(line => line.length > 0);
  if (lines.length < 3) return false;
  const diffLines = lines.filter(line => DIFF_LINE.test(line));
  return diffLines.length >= 3 && diffLines.length / lines.length > 0.4;
}

/* ── Shiki setup ──────────────────────────────────────────────────────────
 * One highlighter, lazily created once. It carries exactly one theme —
 * "github-dark" — used only to make the tokenizer resolve real per-token
 * scope runs (Shiki won't split a line into fine-grained tokens with no
 * theme active at all). Its *colours* are never read: `classifyScope` below
 * maps each token's TextMate scope to one of `index.css`'s `--syn-*`
 * classes, the same ones the diff viewer and the CodeMirror editor already
 * share, so a highlighter swap can never make chat code, diffs, and the
 * editor disagree about colour. */
const SCOPE_THEME = "github-dark";

let corePromise: Promise<HighlighterCore> | null = null;
function core(): Promise<HighlighterCore> {
  corePromise ??= createHighlighterCore({
    engine: createJavaScriptRegexEngine(),
    themes: [import("shiki/themes/github-dark.mjs")],
    langs: [],
  });
  return corePromise;
}

/** The Shiki grammar id to actually load for a public `CANONICAL_LANGS` id. */
function shikiLangFor(language: string): string {
  return SHIKI_GRAMMAR_FOR[language] ?? language;
}

/** Load `shikiLang`'s grammar into the shared highlighter, or `null` if it — or the highlighter itself — fails to load. */
async function highlighterFor(shikiLang: string): Promise<HighlighterCore | null> {
  const loader = LANG_LOADERS[shikiLang];
  if (!loader) return null;
  try {
    const highlighter = await core();
    await highlighter.loadLanguage(loader());
    return highlighter;
  } catch {
    return null;
  }
}

/** Minimal shape we read off a Shiki token — deliberately not importing
 *  Shiki's own token type, so this keeps working across its internal type
 *  re-export churn. */
interface ScopedToken {
  content: string;
  /** Per-scope-run breakdown of the token. Each entry carries its own slice of
   *  `content`, which is what lets a merged token be coloured piecewise —
   *  see `tokenToHtml`. */
  explanation?: { content: string; scopes: { scopeName: string }[] }[];
}

/**
 * Bucket a TextMate scope stack into one of `index.css`'s `.stx-*` classes.
 * Ordered most-specific-scope-first; the first matching prefix wins. This is
 * the same "handful of semantic buckets keyed to one palette" shape as the
 * old `.hljs-*` classes and the editor's `.tok-*` classes — just fed from
 * Shiki's real grammars instead of hljs's lexer or Lezer's parser.
 */
const SCOPE_RULES: [prefix: string, className: string][] = [
  // A comment/string's own delimiters (`/* */`, quote marks) carry their own,
  // more specific `punctuation.definition.*` scope — checked here, before the
  // generic `punctuation` rule below, so `*/` still reads as comment-coloured
  // instead of falling through to plain punctuation.
  ["punctuation.definition.comment", "stx-comment"],
  ["punctuation.definition.string", "stx-string"],
  ["comment", "stx-comment"],
  ["markup.inserted", "stx-addition"],
  ["markup.deleted", "stx-deletion"],
  ["markup.bold", "stx-strong"],
  ["markup.italic", "stx-emphasis"],
  ["markup.strikethrough", "stx-strike"],
  ["markup.heading", "stx-heading"],
  ["markup.underline.link", "stx-link"],
  // Inline code. `punctuation.definition.raw` (the backticks) has to be
  // listed before the generic `punctuation` rule below, the same way comment
  // and string delimiters are, or the ticks come out grey while the run
  // between them — which carries only `markup.inline.raw` — falls through
  // every rule and renders as bare text. One span of literal text, one
  // colour; the editor's Lezer side reaches it through `t.monospace`.
  ["punctuation.definition.raw", "stx-string"],
  ["markup.inline.raw", "stx-string"],
  ["entity.name.section", "stx-heading"],
  // Escapes and regex literals before the generic `string` rule: `\n` inside
  // a string is `constant.character.escape` *nested in* `string`, and the
  // innermost scope is what should win.
  ["constant.character.escape", "stx-regex"],
  ["constant.regexp", "stx-regex"],
  ["string", "stx-string"],
  ["constant.numeric", "stx-number"],
  ["constant.language", "stx-number"],
  ["constant.character", "stx-number"],
  ["constant.other", "stx-number"],
  // `=>` is `storage.type.function.arrow`, which would otherwise hit
  // `storage.type` below and come out keyword-violet. It is an operator, and
  // the editor's Lezer grammar agrees (`function(punctuation)` → operator).
  ["storage.type.function.arrow", "stx-operator"],
  ["storage.type", "stx-keyword"],
  ["storage.modifier", "stx-keyword"],
  // Operators get their own hue rather than sharing punctuation's grey: `=>`,
  // `??` and `===` carry meaning a `;` does not.
  ["keyword.operator", "stx-operator"],
  ["keyword", "stx-keyword"],
  ["entity.name.function", "stx-function"],
  ["entity.name.namespace", "stx-type"],
  ["support.function", "stx-function"],
  ["entity.name.tag", "stx-tag"],
  ["support.class.component", "stx-tag"],
  ["entity.other.attribute-name", "stx-property"],
  ["entity.name.type", "stx-type"],
  ["entity.name.class", "stx-type"],
  ["entity.other.inherited-class", "stx-type"],
  ["support.type", "stx-type"],
  ["support.class", "stx-type"],
  // `.` and `?.` read as structure, not as an operator — kept in the
  // punctuation bucket to match the editor highlighter's `derefOperator`.
  ["punctuation.accessor", "stx-punct"],
  ["punctuation", "stx-punct"],
  // The `variable.*` family, most specific first. Order is load-bearing:
  // `classifyScope` takes the first rule whose prefix matches, so the generic
  // `variable` rule has to come last or it would swallow parameters,
  // constants and `this`.
  ["variable.parameter", "stx-param"],
  // Deliberately *no* `variable.other.constant` rule. TextMate's TypeScript
  // grammar gives that scope to every `const` binding, not to SCREAMING_CASE
  // constants, so bucketing it as a literal painted almost every identifier
  // in a TS file amber. It falls through to `variable` below, which is right.
  ["variable.other.enummember", "stx-number"],
  ["variable.language", "stx-keyword"],
  ["variable.other.property", "stx-property"],
  ["variable.other.object.property", "stx-property"],
  ["support.variable.property", "stx-property"],
  ["meta.object-literal.key", "stx-property"],
  ["variable.function", "stx-function"],
  ["variable", "stx-variable"],
  // Deliberately *no* `meta.decorator` rule, for the same reason C30 has no
  // `meta.function-call` one: it is a *range* scope spanning the whole
  // decorator, so `@Injectable({ scope: 'x' })` had its parens, braces and
  // interior whitespace painted function-blue. `entity.name.function` already
  // covers the callee, and `punctuation.decorator` covers the `@`.
  ["invalid", "stx-invalid"],
];

/**
 * The shared bucket vocabulary: every class name a Bridge syntax renderer may
 * emit. `SCOPE_RULES` above (Shiki, for chat code and diffs) and
 * `editor/highlighter.ts` (Lezer, for the Code tab) both draw from this list,
 * and `palette.test.ts` asserts each entry has a rule in `index.css`. That is
 * what makes it impossible to add a bucket and forget its colour — the failure
 * mode that left `.tok-function` in the stylesheet for months while no
 * renderer could emit it.
 */
export const SYNTAX_CLASSES: string[] = [
  "stx-comment", "stx-keyword", "stx-string", "stx-regex", "stx-number",
  "stx-function", "stx-type", "stx-tag", "stx-property", "stx-variable",
  "stx-param", "stx-operator", "stx-punct", "stx-meta", "stx-invalid",
  "stx-link", "stx-heading", "stx-emphasis", "stx-strong", "stx-strike",
  "stx-addition", "stx-deletion",
];

/**
 * Bucket one scope stack.
 *
 * Innermost scope first, except for regex literals: those are one thing and
 * get one colour from their *container*. Without that exception `/a+b/g`
 * arrives in four colours — the delimiters carry
 * `punctuation.definition.string.*` (string-green), a quantifier carries
 * `keyword.operator.quantifier.regexp` (operator-rose) and the flags carry
 * `keyword.other` (keyword-violet) — while the editor's Lezer grammar tags
 * the whole literal `regexp` and paints it once. The container check is what
 * keeps the two renderers agreeing.
 */
function classifyScopes(scopes: string[]): string | null {
  if (scopes.some(scope => scope === "string.regexp" || scope.startsWith("string.regexp."))) return "stx-regex";
  for (let i = scopes.length - 1; i >= 0; i -= 1) {
    const scope = scopes[i];
    const rule = SCOPE_RULES.find(([prefix]) => scope === prefix || scope.startsWith(`${prefix}.`));
    if (rule) return rule[1];
  }
  return null;
}

function wrap(text: string, className: string | null): string {
  const body = escapeHtml(text);
  return className ? `<span class="${className}">${body}</span>` : body;
}

/**
 * One span per *explanation entry*, not per token.
 *
 * Shiki merges adjacent same-**styled** runs into a single token, and because
 * this file colours by scope rather than by Shiki's theme, a merged token
 * routinely spans several scopes that we want to paint differently. Real
 * examples: `" items."` is one token covering whitespace, an identifier and an
 * accessor; `" alpha; }"` covers an identifier, a terminator and a brace; and
 * `"**bold**"` covers the delimiters *and* the emphasised run.
 *
 * Classifying the whole token from its innermost-last scope therefore painted
 * `items` and `alpha` punctuation-grey, and made `markup.bold`,
 * `markup.italic` and `markup.strikethrough` permanently unreachable, because
 * the closing delimiter always lands last. Each entry carries its own
 * `content`, so splitting there fixes every one of those in one place.
 *
 * The length guard is the safety net: if the entries do not reconstruct the
 * token exactly, fall back to colouring it whole. Dropping or duplicating a
 * character of someone's source is far worse than colouring it bluntly.
 */
function tokenToHtml(token: ScopedToken): string {
  const entries = token.explanation;
  if (entries?.length && entries.reduce((total, entry) => total + entry.content.length, 0) === token.content.length) {
    return entries.map(entry => wrap(entry.content, classifyScopes(entry.scopes.map(scope => scope.scopeName)))).join("");
  }
  const scopes = entries?.flatMap(entry => entry.scopes.map(scope => scope.scopeName)) ?? [];
  return wrap(token.content, classifyScopes(scopes));
}

/** One HTML string per source line, colour-classified via TextMate scopes. */
function linesToHtml(lines: ScopedToken[][]): string[] {
  return lines.map(line => line.map(tokenToHtml).join(""));
}

/** Beyond this, classifying every token's scope costs more than it's worth
 *  (benchmarked: `includeExplanation` typically runs several times slower
 *  than plain tokenization, climbing well past a second on inputs in this
 *  range); render plain. Lower than hljs's old 400,000-char cap because
 *  Shiki's scope classification is measurably heavier than hljs's lexer. */
export const MAX_HIGHLIGHT_CHARS = 50_000;

/**
 * How long a code block's `[lang, body]` must sit still before it's worth
 * colorizing. A streaming reply re-renders `CodeBlock`/`PatchView` on every
 * delta while a fence is still growing — without this, a 200-line fence
 * would schedule ~200 increasingly expensive tokenization passes on its way
 * in, almost all of them for a state the user never gets to see coloured.
 */
export const COLORIZE_DEBOUNCE_MS = 200;

/**
 * Colorize a single code block for `Markdown.tsx`'s `CodeBlock`. Async,
 * because the grammar is a dynamic import — callers render `escapeHtml`
 * plain text immediately and swap this in when it resolves (see
 * `CodeBlock`), the same "on screen now, coloured a frame later" shape
 * `editor/CodeEditor.tsx` already uses for CodeMirror's grammars.
 *
 * An empty or unrecognized `lang` renders plain. hljs used to run
 * `highlightAuto` here — a heuristic best guess across every registered
 * grammar — but Shiki has no equivalent turnkey mode, and running scope
 * classification once per candidate grammar just to score them would multiply
 * the cost this file already spends real effort bounding (see
 * `MAX_HIGHLIGHT_CHARS`, `COLORIZE_DEBOUNCE_MS`). Unlabeled fences losing
 * their guessed colour is an accepted trade-off, not an oversight.
 */
export async function colorizeCode(code: string, lang: string): Promise<string> {
  const language = normalizeLang(lang);
  if (!language || code.length > MAX_HIGHLIGHT_CHARS) return escapeHtml(code);
  const shikiLang = shikiLangFor(language);
  const highlighter = await highlighterFor(shikiLang);
  if (!highlighter) return escapeHtml(code);
  try {
    const { tokens } = highlighter.codeToTokens(code, { lang: shikiLang, theme: SCOPE_THEME, includeExplanation: true });
    return linesToHtml(tokens).join("\n");
  } catch {
    return escapeHtml(code);
  }
}

/* ── Unified diffs ───────────────────────────────────────────────────────── */

export type DiffRowKind = "add" | "del" | "context" | "hunk" | "meta";

/** One rendered diff line: what it is, where it sits, and its (escaped, and
 *  once `colorizePatch` resolves, coloured) body. */
export interface DiffRow {
  kind: DiffRowKind;
  html: string;
  oldLine: number | null;
  newLine: number | null;
}

/** Header lines git emits around a patch — never code, so never highlighted. */
const PATCH_HEADER = /^(diff --git |index |--- |\+\+\+ |old mode |new mode |new file |deleted file |similarity index |dissimilarity index |rename |copy |Binary files |GIT binary patch)/;
/** A `diff --git` at column 0 is unambiguous: a line of code carrying it would
 *  be prefixed by a marker. It is the one header that can also rescue a patch
 *  whose hunk counts lied. */
const FILE_HEADER = /^diff --git /;
const HUNK_HEADER = /^@@+ (?:-(\d+)(?:,(\d+))? )?\+(\d+)(?:,(\d+))? @@/;

interface ParsedPatch {
  rows: DiffRow[];
  oldSide: string[];
  newSide: string[];
  /** Where each row's body lives, so its highlighted text can be pasted back in. */
  source: (({ side: "old" | "new"; index: number }) | null)[];
  language: string;
}

/**
 * Parse a unified diff into rows and the two per-file bodies ("after" and
 * "before") it's made of. Additions and deletions are kept on two separate
 * bodies — an interleaved add/del soup doesn't parse as any real language.
 * Bodies are always plain-escaped here; colour is a separate, async step.
 */
function parsePatch(patch: string, path: string): ParsedPatch {
  const raw = patch.split("\n");
  while (raw.length && raw[raw.length - 1] === "") raw.pop();
  if (!raw.length) return { rows: [], oldSide: [], newSide: [], source: [], language: "" };

  // Measured on the whole patch, not per side: two 25k sides are still half
  // the cap.
  const language = patch.length > MAX_HIGHLIGHT_CHARS ? "" : languageFromPath(path);
  const rows: DiffRow[] = [];
  const oldSide: string[] = [];
  const newSide: string[] = [];
  const source: ParsedPatch["source"] = [];
  let oldNo = 0;
  let newNo = 0;
  // Lines still owed to the current hunk, taken from its own header. Counting
  // them is what ends a hunk — without it, every later file's headers are read
  // as code and lose their first character to the marker slice.
  let oldLeft = 0;
  let newLeft = 0;

  const push = (row: DiffRow, from: ParsedPatch["source"][number]) => {
    rows.push(row);
    source.push(from);
  };
  const meta = (line: string) =>
    push({ kind: "meta", html: escapeHtml(line), oldLine: null, newLine: null }, null);

  for (const line of raw) {
    const hunk = HUNK_HEADER.exec(line);
    if (hunk) {
      oldNo = Number(hunk[1] ?? 0);
      newNo = Number(hunk[3]);
      // A hunk header with no count covers exactly one line.
      oldLeft = hunk[2] === undefined ? 1 : Number(hunk[2]);
      newLeft = hunk[4] === undefined ? 1 : Number(hunk[4]);
      push({ kind: "hunk", html: escapeHtml(line), oldLine: null, newLine: null }, null);
      continue;
    }
    if (FILE_HEADER.test(line)) {
      // Trust the header over a hunk count that has run long.
      oldLeft = 0;
      newLeft = 0;
      meta(line);
      continue;
    }
    const inside = oldLeft > 0 || newLeft > 0;
    // "--- a/x" is a header between hunks and a deleted line of code inside
    // one; "\ No newline at end of file" is neither, and owes no hunk line.
    if (line.startsWith("\\") || (!inside && PATCH_HEADER.test(line))) {
      meta(line);
      continue;
    }
    const marker = line[0] ?? " ";
    if (!inside && marker !== "+" && marker !== "-" && marker !== " ") {
      // Prose wrapped around a fragment ("Success updating foo.ts"). It is not
      // a marker plus a body, so slicing it would eat its first character.
      meta(line);
      continue;
    }
    const body = line.slice(1);
    if (marker === "+") {
      push({ kind: "add", html: escapeHtml(body), oldLine: null, newLine: inside ? newNo++ : null }, { side: "new", index: newSide.length });
      newSide.push(body);
      if (inside) newLeft -= 1;
    } else if (marker === "-") {
      push({ kind: "del", html: escapeHtml(body), oldLine: inside ? oldNo++ : null, newLine: null }, { side: "old", index: oldSide.length });
      oldSide.push(body);
      if (inside) oldLeft -= 1;
    } else {
      push({ kind: "context", html: escapeHtml(body), oldLine: inside ? oldNo++ : null, newLine: inside ? newNo++ : null }, { side: "new", index: newSide.length });
      oldSide.push(body);
      newSide.push(body);
      if (inside) { oldLeft -= 1; newLeft -= 1; }
    }
  }

  return { rows, oldSide, newSide, source, language };
}

/**
 * Parse a unified diff into rows with plain, escaped bodies — synchronous,
 * so `DiffView.tsx`'s `PatchView` can lay out the gutter and row kinds on
 * first render. `colorizePatch` below produces the same rows with the
 * bodies coloured, once the file's grammar has loaded.
 */
export function highlightPatch(patch: string, path = ""): DiffRow[] {
  return parsePatch(patch, path).rows;
}

/** Highlight one side's lines as a single document — so multi-line constructs
 *  (block comments, template literals) still get the tokens they'd have as
 *  real code — then hand back one HTML string per line. */
async function highlightLines(lines: string[], language: string): Promise<string[] | null> {
  if (!lines.length) return [];
  const shikiLang = shikiLangFor(language);
  const highlighter = await highlighterFor(shikiLang);
  if (!highlighter) return null;
  const code = lines.join("\n");
  try {
    const { tokens } = highlighter.codeToTokens(code, { lang: shikiLang, theme: SCOPE_THEME, includeExplanation: true });
    const html = linesToHtml(tokens);
    return html.length === lines.length ? html : null;
  } catch {
    return null;
  }
}

/**
 * The same rows `highlightPatch` returns, with bodies coloured in the file's
 * own language. Async — see `colorizeCode` for why — so `PatchView` renders
 * the plain version first and swaps this in once it resolves.
 */
export async function colorizePatch(patch: string, path = ""): Promise<DiffRow[]> {
  const parsed = parsePatch(patch, path);
  if (!parsed.language) return parsed.rows;
  const [oldHtml, newHtml] = await Promise.all([
    highlightLines(parsed.oldSide, parsed.language),
    highlightLines(parsed.newSide, parsed.language),
  ]);
  if (!oldHtml && !newHtml) return parsed.rows;
  parsed.rows.forEach((row, index) => {
    const from = parsed.source[index];
    if (!from) return;
    const html = from.side === "old" ? oldHtml : newHtml;
    if (html) row.html = html[from.index] ?? row.html;
  });
  return parsed.rows;
}
