import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import c from "highlight.js/lib/languages/c";
import cpp from "highlight.js/lib/languages/cpp";
import csharp from "highlight.js/lib/languages/csharp";
import css from "highlight.js/lib/languages/css";
import dart from "highlight.js/lib/languages/dart";
import diff from "highlight.js/lib/languages/diff";
import dockerfile from "highlight.js/lib/languages/dockerfile";
import elixir from "highlight.js/lib/languages/elixir";
import go from "highlight.js/lib/languages/go";
import graphql from "highlight.js/lib/languages/graphql";
import ini from "highlight.js/lib/languages/ini";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import kotlin from "highlight.js/lib/languages/kotlin";
import lua from "highlight.js/lib/languages/lua";
import makefile from "highlight.js/lib/languages/makefile";
import markdown from "highlight.js/lib/languages/markdown";
import objectivec from "highlight.js/lib/languages/objectivec";
import perl from "highlight.js/lib/languages/perl";
import php from "highlight.js/lib/languages/php";
import protobuf from "highlight.js/lib/languages/protobuf";
import python from "highlight.js/lib/languages/python";
import ruby from "highlight.js/lib/languages/ruby";
import rust from "highlight.js/lib/languages/rust";
import scala from "highlight.js/lib/languages/scala";
import scss from "highlight.js/lib/languages/scss";
import sql from "highlight.js/lib/languages/sql";
import swift from "highlight.js/lib/languages/swift";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

hljs.registerLanguage("bash", bash);
hljs.registerLanguage("c", c);
hljs.registerLanguage("cpp", cpp);
hljs.registerLanguage("csharp", csharp);
hljs.registerLanguage("css", css);
hljs.registerLanguage("dart", dart);
hljs.registerLanguage("diff", diff);
hljs.registerLanguage("dockerfile", dockerfile);
hljs.registerLanguage("elixir", elixir);
hljs.registerLanguage("go", go);
hljs.registerLanguage("graphql", graphql);
hljs.registerLanguage("ini", ini);
hljs.registerLanguage("java", java);
hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("json", json);
hljs.registerLanguage("kotlin", kotlin);
hljs.registerLanguage("lua", lua);
hljs.registerLanguage("makefile", makefile);
hljs.registerLanguage("markdown", markdown);
hljs.registerLanguage("objectivec", objectivec);
hljs.registerLanguage("perl", perl);
hljs.registerLanguage("php", php);
hljs.registerLanguage("protobuf", protobuf);
hljs.registerLanguage("python", python);
hljs.registerLanguage("ruby", ruby);
hljs.registerLanguage("rust", rust);
hljs.registerLanguage("scala", scala);
hljs.registerLanguage("scss", scss);
hljs.registerLanguage("sql", sql);
hljs.registerLanguage("swift", swift);
hljs.registerLanguage("typescript", typescript);
hljs.registerLanguage("xml", xml);
hljs.registerLanguage("yaml", yaml);

const LANG_ALIASES: Record<string, string> = {
  js: "javascript", jsx: "javascript", mjs: "javascript", cjs: "javascript",
  ts: "typescript", tsx: "typescript", mts: "typescript", cts: "typescript",
  py: "python", rs: "rust", sh: "bash", zsh: "bash", shell: "bash", console: "bash",
  yml: "yaml", html: "xml", vue: "xml", svelte: "xml", md: "markdown",
  "c++": "cpp", objc: "objectivec", patch: "diff", toml: "ini", plaintext: "", text: "",
  rb: "ruby", kt: "kotlin", kts: "kotlin", cs: "csharp", ex: "elixir", exs: "elixir",
  gql: "graphql", pl: "perl", sass: "scss", m: "objectivec", mm: "objectivec",
  h: "c", hpp: "cpp", cc: "cpp", cxx: "cpp", proto: "protobuf", make: "makefile",
  jsonc: "json", json5: "json", ndjson: "json", mdx: "markdown",
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

export function normalizeLang(lang: string): string {
  const key = lang.trim().toLowerCase();
  if (!key) return "";
  const aliased = LANG_ALIASES[key] ?? key;
  return hljs.getLanguage(aliased) ? aliased : "";
}

/** Highlight a code block, returning safe HTML (hljs escapes entities). */
export function highlightCode(code: string, lang: string): string {
  const language = normalizeLang(lang);
  try {
    if (language) return hljs.highlight(code, { language, ignoreIllegals: true }).value;
    return hljs.highlightAuto(code).value;
  } catch {
    return escapeHtml(code);
  }
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

export function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/* ── Unified diffs ───────────────────────────────────────────────────────── */

export type DiffRowKind = "add" | "del" | "context" | "hunk" | "meta";

/** One rendered diff line: what it is, where it sits, and its highlighted body. */
export interface DiffRow {
  kind: DiffRowKind;
  /** Highlighted HTML for the line body (marker stripped). Already escaped. */
  html: string;
  /** 1-based line number on each side, or null where the side has no line. */
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

/** Beyond this, highlighting a patch costs more than it's worth; render plain. */
const MAX_HIGHLIGHT_CHARS = 400_000;

/**
 * Split hljs output into one HTML string per line, re-opening any span that
 * straddles a newline. Highlighting the whole side at once is what keeps
 * multi-line constructs (block comments, template literals) intact; this puts
 * the result back into rows we can colour and number individually.
 */
export function splitHighlightedLines(html: string): string[] {
  const lines: string[] = [];
  const open: string[] = [];
  let current = "";
  for (const token of html.match(/<span[^>]*>|<\/span>|[^<]+/g) ?? []) {
    if (token.startsWith("</")) {
      open.pop();
      current += token;
    } else if (token.startsWith("<")) {
      open.push(token);
      current += token;
    } else {
      const parts = token.split("\n");
      parts.forEach((part, index) => {
        if (index > 0) {
          current += "</span>".repeat(open.length);
          lines.push(current);
          current = open.join("");
        }
        current += part;
      });
    }
  }
  lines.push(current);
  return lines;
}

/** Highlight lines as one document, falling back to plain text on any mismatch. */
function highlightSide(lines: string[], language: string): string[] {
  if (!lines.length) return [];
  if (!language) return lines.map(escapeHtml);
  const code = lines.join("\n");
  if (code.length > MAX_HIGHLIGHT_CHARS) return lines.map(escapeHtml);
  try {
    const split = splitHighlightedLines(hljs.highlight(code, { language, ignoreIllegals: true }).value);
    if (split.length === lines.length) return split;
  } catch {
    // fall through
  }
  return lines.map(escapeHtml);
}

/**
 * Parse a unified diff into rows whose bodies are highlighted in the file's own
 * language. Additions and deletions are highlighted as two separate documents —
 * the "after" file and the "before" file — so each side parses as real code
 * instead of as an interleaved soup that no grammar can make sense of.
 */
export function highlightPatch(patch: string, path = ""): DiffRow[] {
  const raw = patch.split("\n");
  while (raw.length && raw[raw.length - 1] === "") raw.pop();
  if (!raw.length) return [];

  // Measured on the whole patch, not per side: two 250k sides are still half a
  // megabyte of parsing.
  const language = patch.length > MAX_HIGHLIGHT_CHARS ? "" : languageFromPath(path);
  const rows: DiffRow[] = [];
  const oldSide: string[] = [];
  const newSide: string[] = [];
  // Where each row's body lives, so we can paste highlighted text back in.
  const source: Array<{ side: "old" | "new"; index: number } | null> = [];
  let oldNo = 0;
  let newNo = 0;
  // Lines still owed to the current hunk, taken from its own header. Counting
  // them is what ends a hunk — without it, every later file's headers are read
  // as code and lose their first character to the marker slice.
  let oldLeft = 0;
  let newLeft = 0;

  const push = (row: DiffRow, from: { side: "old" | "new"; index: number } | null) => {
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
      push({ kind: "add", html: "", oldLine: null, newLine: inside ? newNo++ : null }, { side: "new", index: newSide.length });
      newSide.push(body);
      if (inside) newLeft -= 1;
    } else if (marker === "-") {
      push({ kind: "del", html: "", oldLine: inside ? oldNo++ : null, newLine: null }, { side: "old", index: oldSide.length });
      oldSide.push(body);
      if (inside) oldLeft -= 1;
    } else {
      push({ kind: "context", html: "", oldLine: inside ? oldNo++ : null, newLine: inside ? newNo++ : null }, { side: "new", index: newSide.length });
      oldSide.push(body);
      newSide.push(body);
      if (inside) { oldLeft -= 1; newLeft -= 1; }
    }
  }

  const highlighted = { old: highlightSide(oldSide, language), new: highlightSide(newSide, language) };
  rows.forEach((row, index) => {
    const from = source[index];
    if (from) row.html = highlighted[from.side][from.index] ?? "";
  });
  return rows;
}
